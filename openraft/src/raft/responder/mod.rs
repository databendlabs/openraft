//! API to consumer a response when a client write request is completed.

pub(crate) mod builder;
pub(crate) mod either;
pub(crate) mod impls;
pub use builder::ResponderBuilder;
pub use impls::OneshotResponder;

use crate::OptionalSend;

/// A trait that lets `RaftCore` send a result back to the client or to somewhere else.
///
/// This is a generic abstraction for sending results of any type `T`.
/// It is created for each request and is sent to `RaftCore`.
/// Once the request is completed, the `RaftCore` sends the result via it.
/// The implementation of the trait then forwards the response to the application.
///
/// Usually an implementation of [`Responder`] is a oneshot channel Sender.
///
/// See [`ResponderBuilder`] for constructing responders and their receivers.
///
/// # Type Parameters
///
/// - `T`: The type of value to send through this responder
pub trait Responder<T>: OptionalSend + 'static {
    /// Send result when the request has been completed.
    ///
    /// This method is called by the `RaftCore` once the request has been processed.
    fn send(self, result: T);
}
