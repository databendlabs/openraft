//! Bounded MPSC channel traits.

use std::future::Future;

use base::OptionalSend;
use base::OptionalSync;

/// mpsc shares the same error types as mpsc_unbounded
pub use super::mpsc_unbounded::SendError;
pub use super::mpsc_unbounded::TryRecvError;
use crate::base;

/// Multi-producer, single-consumer channel.
pub trait Mpsc: Sized + OptionalSend {
    /// The sender type for this MPSC channel.
    type Sender<T: OptionalSend>: MpscSender<Self, T>;
    /// The receiver type for this MPSC channel.
    type Receiver<T: OptionalSend>: MpscReceiver<T>;
    /// The weak sender type for this MPSC channel.
    type WeakSender<T: OptionalSend>: MpscWeakSender<Self, T>;

    /// Creates a bounded mpsc channel for communicating between asynchronous tasks with
    /// backpressure.
    fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>);
}

/// Send values to the associated [`MpscReceiver`].
pub trait MpscSender<MU, T>: OptionalSend + OptionalSync + Clone
where
    MU: Mpsc,
    T: OptionalSend,
{
    /// Attempts to send a message, blocks if there is no capacity.
    ///
    /// If the receiving half of the channel is closed, this
    /// function returns an error. The error includes the value passed to `send`.
    fn send(&self, msg: T) -> impl Future<Output = Result<(), SendError<T>>> + OptionalSend;

    /// Converts the [`MpscSender`] to a [`MpscWeakSender`] that does not count
    /// towards RAII semantics, i.e., if all `Sender` instances of the
    /// channel were dropped and only `WeakSender` instances remain,
    /// the channel is closed.
    fn downgrade(&self) -> MU::WeakSender<T>;
}

/// Receive values from the associated [`MpscSender`].
pub trait MpscReceiver<T>: OptionalSend + OptionalSync {
    /// Receives the next value for this receiver.
    ///
    /// This method returns `None` if the channel has been closed and there are
    /// no remaining messages in the channel's buffer.
    fn recv(&mut self) -> impl Future<Output = Option<T>> + OptionalSend;

    /// Tries to receive the next value for this receiver.
    ///
    /// This method returns the [`TryRecvError::Empty`] error if the channel is currently
    /// empty, but there are still outstanding senders.
    ///
    /// This method returns the [`TryRecvError::Disconnected`] error if the channel is
    /// currently empty, and there are no outstanding senders.
    fn try_recv(&mut self) -> Result<T, TryRecvError>;
}

/// A sender that does not prevent the channel from being closed.
///
/// If all [`MpscSender`] instances of a channel were dropped and only
/// `WeakSender` instances remain, the channel is closed.
pub trait MpscWeakSender<MU, T>: OptionalSend + OptionalSync + Clone
where
    MU: Mpsc,
    T: OptionalSend,
{
    /// Tries to convert a [`MpscWeakSender`] into an [`MpscSender`].
    ///
    /// This will return `Some` if there are other `Sender` instances alive and
    /// the channel wasn't previously dropped, otherwise `None` is returned.
    fn upgrade(&self) -> Option<MU::Sender<T>>;
}
