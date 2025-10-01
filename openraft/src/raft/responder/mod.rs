//! API to consumer a response when a client write request is completed.

pub(crate) mod either;
pub(crate) mod impls;
pub use impls::OneshotResponder;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::raft::message::ClientWriteResult;

/// A trait that lets `RaftCore` send the response or an error of a client write request back to the
/// client or to somewhere else.
///
/// It is created for each request [`AppData`] and is sent to `RaftCore`.
/// Once the request is completed,
/// the `RaftCore` sends the result [`ClientWriteResult`] via it.
/// The implementation of the trait then forwards the response to the application.
/// There could optionally be a receiver to wait for the response.
///
/// Usually an implementation of [`Responder`] is a oneshot channel Sender,
/// and [`Responder::Receiver`] is a oneshot channel Receiver.
///
/// [`AppData`]: `crate::AppData`
pub trait Responder<C>: OptionalSend + 'static
where C: RaftTypeConfig
{
    /// An optional receiver to receive the result sent by `RaftCore`.
    ///
    /// If the application does not need to wait for the response, it can be `()`.
    type Receiver;

    /// Build a new instance from the application request.
    fn from_app_data(app_data: C::D) -> (C::D, Self, Self::Receiver)
    where Self: Sized;

    /// Send result when the request has been completed.
    ///
    /// This method is called by the `RaftCore` once the request has been applied to state machine.
    fn send(self, result: ClientWriteResult<C>);
}
