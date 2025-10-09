//! Builder trait for creating Responders.

use crate::OptionalSend;
use crate::raft::responder::Responder;

/// A trait for building [`Responder`] instances from source data.
///
/// This trait separates the construction logic from the behavior of a responder.
/// Different implementations can build responders from different sources:
/// - `()` for responders that need no context
/// - Application data for responders that need request context
/// - Custom types for responders that need other context
///
/// # Type Parameters
///
/// - `T`: The type of data needed to build the responder
/// - `R`: The type of result that the responder will send
///
/// [`Responder`]: crate::raft::responder::Responder
pub trait ResponderBuilder<T, R = ()>: OptionalSend {
    /// The type of responder this builder creates.
    type Responder: Responder<R> + OptionalSend;

    /// An optional receiver to receive the result sent by `RaftCore`.
    ///
    /// If the application does not need to wait for the response, it can be `()`.
    type Receiver;

    /// Build a new responder and its receiver from the source data.
    ///
    /// Returns a tuple of (responder, receiver) where the responder will be sent to
    /// RaftCore and the receiver can be used to wait for the response.
    fn build(src: &T) -> (Self::Responder, Self::Receiver);
}
