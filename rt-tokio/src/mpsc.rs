use std::future::Future;

use futures_util::TryFutureExt;
use openraft_rt::Mpsc;
use openraft_rt::MpscReceiver;
use openraft_rt::MpscSender;
use openraft_rt::MpscWeakSender;
use openraft_rt::OptionalSend;
use openraft_rt::SendError;
use openraft_rt::TryRecvError;
use tokio::sync::mpsc;

pub struct TokioMpsc;

/// Wrapper around `tokio::sync::mpsc::Sender` to implement the `MpscSender` trait.
pub struct TokioMpscSender<T>(mpsc::Sender<T>);

/// Wrapper around `tokio::sync::mpsc::Receiver` to implement the `MpscReceiver` trait.
pub struct TokioMpscReceiver<T>(mpsc::Receiver<T>);

/// Wrapper around `tokio::sync::mpsc::WeakSender` to implement the `MpscWeakSender` trait.
pub struct TokioMpscWeakSender<T>(mpsc::WeakSender<T>);

impl<T> Clone for TokioMpscSender<T> {
    fn clone(&self) -> Self {
        TokioMpscSender(self.0.clone())
    }
}

impl<T> Clone for TokioMpscWeakSender<T> {
    fn clone(&self) -> Self {
        TokioMpscWeakSender(self.0.clone())
    }
}

impl Mpsc for TokioMpsc {
    type Sender<T: OptionalSend> = TokioMpscSender<T>;
    type Receiver<T: OptionalSend> = TokioMpscReceiver<T>;
    type WeakSender<T: OptionalSend> = TokioMpscWeakSender<T>;

    /// Creates a bounded mpsc channel for communicating between asynchronous
    /// tasks with backpressure.
    fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = mpsc::channel(buffer);
        (TokioMpscSender(tx), TokioMpscReceiver(rx))
    }
}

impl<T> MpscSender<TokioMpsc, T> for TokioMpscSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(&self, msg: T) -> impl Future<Output = Result<(), SendError<T>>> + OptionalSend {
        self.0.send(msg).map_err(|e| SendError(e.0))
    }

    #[inline]
    fn downgrade(&self) -> <TokioMpsc as Mpsc>::WeakSender<T> {
        TokioMpscWeakSender(self.0.downgrade())
    }
}

impl<T> MpscReceiver<T> for TokioMpscReceiver<T>
where T: OptionalSend
{
    #[inline]
    fn recv(&mut self) -> impl Future<Output = Option<T>> + OptionalSend {
        self.0.recv()
    }

    #[inline]
    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.0.try_recv().map_err(|e| match e {
            mpsc::error::TryRecvError::Empty => TryRecvError::Empty,
            mpsc::error::TryRecvError::Disconnected => TryRecvError::Disconnected,
        })
    }
}

impl<T> MpscWeakSender<TokioMpsc, T> for TokioMpscWeakSender<T>
where T: OptionalSend
{
    #[inline]
    fn upgrade(&self) -> Option<<TokioMpsc as Mpsc>::Sender<T>> {
        self.0.upgrade().map(TokioMpscSender)
    }
}
