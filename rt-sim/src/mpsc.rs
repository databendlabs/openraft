//! Bounded MPSC channels, wrapping `tokio::sync::mpsc`.
//!
//! The tokio channel needs no tokio runtime and wakes waiters in FIFO order, so it is reused as is.

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

/// Bounded MPSC channel.
pub struct SimMpsc;

/// Wrapper around `tokio::sync::mpsc::Sender` to implement the `MpscSender` trait.
pub struct SimMpscSender<T>(mpsc::Sender<T>);

/// Wrapper around `tokio::sync::mpsc::Receiver` to implement the `MpscReceiver` trait.
pub struct SimMpscReceiver<T>(mpsc::Receiver<T>);

/// Wrapper around `tokio::sync::mpsc::WeakSender` to implement the `MpscWeakSender` trait.
pub struct SimMpscWeakSender<T>(mpsc::WeakSender<T>);

impl<T> Clone for SimMpscSender<T> {
    fn clone(&self) -> Self {
        SimMpscSender(self.0.clone())
    }
}

impl<T> Clone for SimMpscWeakSender<T> {
    fn clone(&self) -> Self {
        SimMpscWeakSender(self.0.clone())
    }
}

impl Mpsc for SimMpsc {
    type Sender<T: OptionalSend> = SimMpscSender<T>;
    type Receiver<T: OptionalSend> = SimMpscReceiver<T>;
    type WeakSender<T: OptionalSend> = SimMpscWeakSender<T>;

    /// Creates a bounded mpsc channel for communicating between asynchronous
    /// tasks with backpressure.
    fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
        let (tx, rx) = mpsc::channel(buffer);
        (SimMpscSender(tx), SimMpscReceiver(rx))
    }
}

impl<T> MpscSender<SimMpsc, T> for SimMpscSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(&self, msg: T) -> impl Future<Output = Result<(), SendError<T>>> + OptionalSend {
        self.0.send(msg).map_err(|e| SendError(e.0))
    }

    #[inline]
    fn downgrade(&self) -> <SimMpsc as Mpsc>::WeakSender<T> {
        SimMpscWeakSender(self.0.downgrade())
    }
}

impl<T> MpscReceiver<T> for SimMpscReceiver<T>
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

impl<T> MpscWeakSender<SimMpsc, T> for SimMpscWeakSender<T>
where T: OptionalSend
{
    #[inline]
    fn upgrade(&self) -> Option<<SimMpsc as Mpsc>::Sender<T>> {
        self.0.upgrade().map(SimMpscSender)
    }
}
