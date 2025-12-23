use std::future::Future;

use futures::TryFutureExt;
use tokio::sync::mpsc;

use crate::OptionalSend;
use crate::async_runtime::Mpsc;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::MpscWeakSender;
use crate::async_runtime::SendError;
use crate::async_runtime::TryRecvError;

pub struct TokioMpsc;

impl Mpsc for TokioMpsc {
    type Sender<T: OptionalSend> = mpsc::Sender<T>;
    type Receiver<T: OptionalSend> = mpsc::Receiver<T>;
    type WeakSender<T: OptionalSend> = mpsc::WeakSender<T>;

    /// Creates a bounded mpsc channel for communicating between asynchronous
    /// tasks with backpressure.
    fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        mpsc::channel(buffer)
    }
}

impl<T> MpscSender<TokioMpsc, T> for mpsc::Sender<T>
where T: OptionalSend
{
    #[inline]
    fn send(&self, msg: T) -> impl Future<Output = Result<(), SendError<T>>> + OptionalSend {
        self.send(msg).map_err(|e| SendError(e.0))
    }

    #[inline]
    fn downgrade(&self) -> <TokioMpsc as Mpsc>::WeakSender<T> {
        self.downgrade()
    }
}

impl<T> MpscReceiver<T> for mpsc::Receiver<T>
where T: OptionalSend
{
    #[inline]
    fn recv(&mut self) -> impl Future<Output = Option<T>> + OptionalSend {
        self.recv()
    }

    #[inline]
    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.try_recv().map_err(|e| match e {
            mpsc::error::TryRecvError::Empty => TryRecvError::Empty,
            mpsc::error::TryRecvError::Disconnected => TryRecvError::Disconnected,
        })
    }
}

impl<T> MpscWeakSender<TokioMpsc, T> for mpsc::WeakSender<T>
where T: OptionalSend
{
    #[inline]
    fn upgrade(&self) -> Option<<TokioMpsc as Mpsc>::Sender<T>> {
        self.upgrade()
    }
}
