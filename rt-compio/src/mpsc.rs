use std::future::Future;

use futures::TryFutureExt;
use openraft::async_runtime::Mpsc;
use openraft::async_runtime::MpscReceiver;
use openraft::async_runtime::MpscSender;
use openraft::async_runtime::MpscWeakSender;
use openraft::async_runtime::SendError;
use openraft::async_runtime::TryRecvError;
use openraft::OptionalSend;
use tokio::sync::mpsc as tokio_mpsc;

pub struct CompioMpsc;

pub struct CompioMpscSender<T>(tokio_mpsc::Sender<T>);

impl<T> Clone for CompioMpscSender<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct CompioMpscReceiver<T>(tokio_mpsc::Receiver<T>);

pub struct CompioMpscWeakSender<T>(tokio_mpsc::WeakSender<T>);

impl<T> Clone for CompioMpscWeakSender<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Mpsc for CompioMpsc {
    type Sender<T: OptionalSend> = CompioMpscSender<T>;
    type Receiver<T: OptionalSend> = CompioMpscReceiver<T>;
    type WeakSender<T: OptionalSend> = CompioMpscWeakSender<T>;

    #[inline]
    fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = tokio_mpsc::channel(buffer);
        let tx_wrapper = CompioMpscSender(tx);
        let rx_wrapper = CompioMpscReceiver(rx);

        (tx_wrapper, rx_wrapper)
    }
}

impl<T> MpscSender<CompioMpsc, T> for CompioMpscSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(&self, msg: T) -> impl Future<Output = Result<(), SendError<T>>> {
        self.0.send(msg).map_err(|e| SendError(e.0))
    }

    #[inline]
    fn downgrade(&self) -> <CompioMpsc as Mpsc>::WeakSender<T> {
        let inner = self.0.downgrade();
        CompioMpscWeakSender(inner)
    }
}

impl<T> MpscReceiver<T> for CompioMpscReceiver<T> {
    #[inline]
    fn recv(&mut self) -> impl Future<Output = Option<T>> {
        self.0.recv()
    }

    #[inline]
    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.0.try_recv().map_err(|e| match e {
            tokio_mpsc::error::TryRecvError::Empty => TryRecvError::Empty,
            tokio_mpsc::error::TryRecvError::Disconnected => TryRecvError::Disconnected,
        })
    }
}

impl<T> MpscWeakSender<CompioMpsc, T> for CompioMpscWeakSender<T>
where T: OptionalSend
{
    #[inline]
    fn upgrade(&self) -> Option<<CompioMpsc as Mpsc>::Sender<T>> {
        self.0.upgrade().map(CompioMpscSender)
    }
}
