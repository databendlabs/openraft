use openraft_macros::since;

use crate::AsyncRuntime;
use crate::CancelHandle;
use crate::Mpsc;
use crate::MpscReceiver;
use crate::OptionalSend;
use crate::TryRecvError;

mod buffering_task;

use buffering_task::BufferingTask;

/// A receiver whose background task buffers messages without a backlog limit.
///
/// The task keeps draining the source while the consumer is busy. Messages retain their
/// receive order. Dropping this receiver stops the task and drops the source receiver.
/// After all source senders close, buffered messages drain before [`Self::recv`] returns `None`.
/// Memory use grows when producers outpace the consumer.
#[since(version = "0.10.0")]
pub struct BufferedReceiver<Rt, T>
where
    Rt: AsyncRuntime,
    T: OptionalSend + 'static,
{
    receiver: <Rt::Mpsc as Mpsc>::Receiver<T>,
    _cancel: CancelHandle<Rt>,
    _task: Rt::JoinHandle<()>,
}

impl<Rt, T> BufferedReceiver<Rt, T>
where
    Rt: AsyncRuntime,
    T: OptionalSend + 'static,
{
    pub(crate) fn new<R>(source: R) -> Self
    where R: MpscReceiver<T> + 'static {
        let (sender, receiver) = Rt::Mpsc::channel(1);
        let task = BufferingTask::<R, T>::new(source);
        let future = task.run::<Rt>(sender);
        let (cancel, future) = Rt::cancellable(future);
        let task = Rt::spawn(async move {
            let _ = future.await;
        });
        Self {
            receiver,
            _cancel: cancel,
            _task: task,
        }
    }
}

impl<Rt, T> MpscReceiver<T> for BufferedReceiver<Rt, T>
where
    Rt: AsyncRuntime,
    T: OptionalSend + 'static,
{
    async fn recv(&mut self) -> Option<T> {
        self.receiver.recv().await
    }

    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.receiver.try_recv()
    }
}
