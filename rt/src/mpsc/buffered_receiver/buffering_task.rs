use std::collections::VecDeque;
use std::future::Future;
use std::future::pending;
use std::pin::pin;

use crate::AsyncRuntime;
use crate::Mpsc;
use crate::MpscReceiver;
use crate::MpscSender;
use crate::Oneshot;
use crate::OptionalSend;
use crate::Select2;
use crate::Select3;
use crate::SendError;

pub(super) struct BufferingTask<Rt, R, T>
where
    Rt: AsyncRuntime,
    R: MpscReceiver<T>,
    T: OptionalSend,
{
    source: Option<R>,
    queued: VecDeque<T>,
    cancelled: <Rt::Oneshot as Oneshot>::Receiver<()>,
}

impl<Rt, R, T> BufferingTask<Rt, R, T>
where
    Rt: AsyncRuntime,
    R: MpscReceiver<T>,
    T: OptionalSend,
{
    pub(super) fn new(source: R, cancelled: <Rt::Oneshot as Oneshot>::Receiver<()>) -> Self {
        Self {
            source: Some(source),
            queued: VecDeque::new(),
            cancelled,
        }
    }

    pub(super) async fn run(mut self, sender: <Rt::Mpsc as Mpsc>::Sender<T>) {
        while let Some(command) = self.next().await {
            let send = sender.send(command);
            if !self.forward(send).await {
                return;
            }
        }
    }

    async fn next(&mut self) -> Option<T> {
        if let Some(command) = self.queued.pop_front() {
            return Some(command);
        }
        self.source.as_ref()?;
        let receive = Self::receive(&mut self.source);
        let selected = Rt::select2(&mut self.cancelled, receive).await;
        match selected {
            Select2::First(_) => None,
            Select2::Second(command) => command,
        }
    }

    async fn forward<F>(&mut self, send: F) -> bool
    where F: Future<Output = Result<(), SendError<T>>> + OptionalSend {
        let mut send = pin!(send);
        loop {
            let receive = Self::receive(&mut self.source);
            let selected = Rt::select3(&mut self.cancelled, send.as_mut(), receive).await;
            match selected {
                Select3::First(_) => return false,
                Select3::Second(sent) => return sent.is_ok(),
                Select3::Third(Some(command)) => self.queued.push_back(command),
                Select3::Third(None) => self.source = None,
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
