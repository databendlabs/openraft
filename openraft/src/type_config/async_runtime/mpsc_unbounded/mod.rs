mod send_error;
mod try_recv_error;

pub use send_error::SendError;
pub use try_recv_error::TryRecvError;

use crate::OptionalSend;
use crate::OptionalSync;

pub trait MpscUnbounded: Sized + OptionalSend {
    type Sender<T: OptionalSend>: MpscUnboundedSender<Self, T>;
    type Receiver<T: OptionalSend>: MpscUnboundedReceiver<T>;
    type WeakSender<T: OptionalSend>: MpscUnboundedWeakSender<Self, T>;

    fn channel<T: OptionalSend>() -> (Self::Sender<T>, Self::Receiver<T>);
}

/// Send values to the associated [`MpscUnboundedReceiver`].
pub trait MpscUnboundedSender<MU, T>: OptionalSend + OptionalSync + Clone
where
    MU: MpscUnbounded,
    T: OptionalSend,
{
    /// Attempts to send a message without blocking.
    ///
    /// If the receiving half of the channel is closed, this
    /// function returns an error. The error includes the value passed to `send`.
    fn send(&self, msg: T) -> Result<(), SendError<T>>;

    /// Converts the [`MpscUnboundedSender`] to a [`MpscUnboundedWeakSender`] that does not count
    /// towards RAII semantics, i.e., if all `Sender` instances of the
    /// channel were dropped and only `WeakSender` instances remain,
    /// the channel is closed.
    fn downgrade(&self) -> MU::WeakSender<T>;
}

/// Receive values from the associated [`MpscUnboundedSender`].
pub trait MpscUnboundedReceiver<T>: OptionalSend + OptionalSync {
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
/// If all [`MpscUnboundedSender`] instances of a channel were dropped and only
/// `WeakSender` instances remain, the channel is closed.
pub trait MpscUnboundedWeakSender<MU, T>: OptionalSend + OptionalSync + Clone
where
    MU: MpscUnbounded,
    T: OptionalSend,
{
    /// Tries to convert a [`MpscUnboundedWeakSender`] into an [`MpscUnboundedSender`].
    ///
    /// This will return `Some` if there are other `Sender` instances alive and
    /// the channel wasn't previously dropped, otherwise `None` is returned.
    fn upgrade(&self) -> Option<MU::Sender<T>>;
}
