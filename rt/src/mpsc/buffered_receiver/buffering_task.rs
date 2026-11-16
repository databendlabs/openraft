use std::collections::VecDeque;
use std::future::Future;
use std::future::pending;
use std::pin::pin;

use crate::AsyncRuntime;
use crate::Mpsc;
use crate::MpscReceiver;
use crate::MpscSender;
use crate::OptionalSend;
use crate::Select2;
use crate::SendError;

pub(super) struct BufferingTask<R, T>
where
    R: MpscReceiver<T>,
    T: OptionalSend,
{
    source: Option<R>,
    queued: VecDeque<T>,
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

    pub(super) async fn run<Rt>(mut self, sender: <Rt::Mpsc as Mpsc>::Sender<T>)
    where Rt: AsyncRuntime {
        while let Some(command) = self.next().await {
            let send = sender.send(command);
            let forwarded = self.forward::<Rt, _>(send).await;
            if !forwarded {
                return;
            }
        }
    }

    async fn next(&mut self) -> Option<T> {
        if let Some(command) = self.queued.pop_front() {
            return Some(command);
        }
        let source = self.source.as_mut()?;
        source.recv().await
    }

    async fn forward<Rt, F>(&mut self, send: F) -> bool
    where
        Rt: AsyncRuntime,
        F: Future<Output = Result<(), SendError<T>>> + OptionalSend,
    {
        let mut send = pin!(send);
        loop {
            let receive = Self::receive(&mut self.source);
            let selected = Rt::select2(send.as_mut(), receive).await;
            match selected {
                Select2::First(sent) => return sent.is_ok(),
                Select2::Second(Some(command)) => self.queued.push_back(command),
                Select2::Second(None) => self.source = None,
            }
        }
    }

    async fn receive(source: &mut Option<R>) -> Option<T> {
        let Some(source) = source else {
            return pending().await;
        };
        source.recv().await
    }
}
