use std::collections::VecDeque;
use std::future::poll_fn;
use std::pin::pin;
use std::task::Poll;

use crate::AsyncRuntime;
use crate::Mpsc;
use crate::MpscReceiver;
use crate::MpscSender;
use crate::OptionalSend;

/// How many events [`BufferingTask::run`] handles in one poll before it lets other tasks run.
///
/// When the source is always ready, every receive completes at once. Without a limit, the task
/// would drain the whole source in one poll while a consumer it has already woken waits.
const EVENTS_PER_POLL: usize = 32;

pub(super) struct BufferingTask<R, T>
where
    R: MpscReceiver<T>,
    T: OptionalSend,
{
    source: Option<R>,
    queued: VecDeque<T>,
}

/// What completed in one turn of [`BufferingTask::run`].
enum Event<R, T> {
    /// The send of the oldest queued command finished; `false` when the output receiver is gone.
    Sent(bool),
    /// The source receive finished, with the source returned and its command, `None` once closed.
    Received(R, Option<T>),
}

impl<R, T> BufferingTask<R, T>
where
    R: MpscReceiver<T>,
    T: OptionalSend,
{
    pub(super) fn new(source: R) -> Self {
        Self {
            source: Some(source),
            queued: VecDeque::new(),
        }
    }

    /// Forward commands from the source to `sender`, buffering them while the output is full.
    ///
    /// The send of the oldest queued command and the receive of the next source command stay in
    /// flight until each completes: dropping a receive that is already pending could lose a
    /// command it has taken from the source. The send is polled first when both are ready.
    ///
    /// The task handles at most [`EVENTS_PER_POLL`] events in one poll. Then it wakes itself and
    /// returns `Pending` once, so that the executor can run a consumer before the task reads
    /// further ahead. The queue stays unbounded, and the send and receive in flight are kept
    /// across that yield.
    pub(super) async fn run<Rt>(self, sender: <Rt::Mpsc as Mpsc>::Sender<T>)
    where Rt: AsyncRuntime {
        let Self { source, mut queued } = self;
        let send = |command: T| sender.send(command);

        let mut sending = pin!(None.map(send));
        let mut receiving = pin!(source.map(Self::receive));
        let mut budget = EVENTS_PER_POLL;

        loop {
            if sending.is_none() {
                match queued.pop_front() {
                    Some(command) => sending.set(Some(send(command))),
                    None if receiving.is_none() => return,
                    None => {}
                }
            }

            if budget == 0 {
                yield_now().await;
                budget = EVENTS_PER_POLL;
            }

            let event = poll_fn(|cx| {
                if let Some(send) = sending.as_mut().as_pin_mut()
                    && let Poll::Ready(sent) = send.poll(cx)
                {
                    return Poll::Ready(Event::Sent(sent.is_ok()));
                }
                if let Some(receive) = receiving.as_mut().as_pin_mut()
                    && let Poll::Ready((source, command)) = receive.poll(cx)
                {
                    return Poll::Ready(Event::Received(source, command));
                }
                // The task suspends here, so the poll that resumes it starts a new batch.
                budget = EVENTS_PER_POLL;
                Poll::Pending
            })
            .await;
            budget -= 1;

            match event {
                Event::Sent(true) => sending.set(None),
                Event::Sent(false) => return,
                Event::Received(source, Some(command)) => {
                    queued.push_back(command);
                    receiving.set(Some(Self::receive(source)));
                }
                Event::Received(_closed, None) => receiving.set(None),
            }
        }
    }

    /// Receive the next command, and give the source back with it.
    async fn receive(mut source: R) -> (R, Option<T>) {
        let command = source.recv().await;
        (source, command)
    }
}

/// Return `Pending` once after waking the current task, so that the executor can run other tasks
/// before it polls this one again.
async fn yield_now() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            return Poll::Ready(());
        }
        yielded = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    })
    .await
}
