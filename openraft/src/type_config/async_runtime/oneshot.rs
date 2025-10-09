//! Oneshot channel traits.

use std::future::Future;

use crate::OptionalSend;
use crate::OptionalSync;

/// A oneshot channel for sending a single value between asynchronous tasks.
pub trait Oneshot {
    /// Type of a `oneshot` sender.
    type Sender<T: OptionalSend>: OneshotSender<T>;
    /// Type of a `oneshot` receiver.
    type Receiver<T: OptionalSend>: OptionalSend
        + OptionalSync
        + Future<Output = Result<T, Self::ReceiverError>>
        + Unpin;
    /// Type of a `oneshot` receiver error.
    type ReceiverError: std::error::Error + OptionalSend;

    /// Creates a new one-shot channel for sending single values.
    ///
    /// The function returns separate "send" and "receive" handles. The `Sender`
    /// handle is used by the producer to send the value. The `Receiver` handle is
    /// used by the consumer to receive the value.
    ///
    /// Each handle can be used on separate tasks.
    fn channel<T>() -> (Self::Sender<T>, Self::Receiver<T>)
    where T: OptionalSend;
}

/// Send a value on a oneshot channel.
pub trait OneshotSender<T>: OptionalSend + OptionalSync + Sized
where T: OptionalSend
{
    /// Attempts to send a value on this channel, returning it back if it could
    /// not be sent.
    ///
    /// This method consumes `self` as only one value may ever be sent on a `oneshot`
    /// channel. It is not marked async because sending a message to an `oneshot`
    /// channel never requires any form of waiting.  Because of this, the `send`
    /// method can be used in both synchronous and asynchronous code without
    /// problems.
    fn send(self, t: T) -> Result<(), T>;
}
