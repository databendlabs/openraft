//! API to consume a response when a client write request is completed.

pub(crate) mod core_responder;
pub(crate) mod impls;
pub use impls::OneshotResponder;
pub use impls::ProgressResponder;
use openraft_macros::since;

use crate::LogId;
use crate::OptionalSend;
use crate::RaftTypeConfig;

/// A trait that lets `RaftCore` send a result back to the client or to somewhere else.
///
/// This is a generic abstraction for sending results of any type `T`.
/// Usually an implementation of [`Responder`] is a oneshot channel Sender.
///
/// ## Lifecycle Callbacks
///
/// - [`on_commit()`](Self::on_commit): Called when locally committed (optional)
/// - [`on_complete()`](Self::on_complete): Sends the final result
///
/// # Type Parameters
///
/// - `T`: The type of value to send through this responder
pub trait Responder<C, T>
where
    Self: OptionalSend + Sized + 'static,
    C: RaftTypeConfig,
{
    /// Called when the log entry is locally committed (safe to read).
    ///
    /// Invoked when the log has been replicated to a quorum. At this point, the log is guaranteed
    /// to be visible to all future leaders and can be read immediately.
    ///
    /// # Parameters
    ///
    /// - `log_id`: The log ID assigned by the proposing leader.
    ///
    /// Default implementation does nothing.
    #[since(version = "0.10.0")]
    fn on_commit(&mut self, _log_id: LogId<C>) {}

    /// Called when the request completes (applied; previously it is `send`).
    /// Send the final result to the client.
    ///
    /// Invoked in two scenarios:
    /// - **Normal**: Log entry applied to the state machine
    /// - **Early termination**: Request failed (e.g., `ForwardToLeader` error)
    #[since(version = "0.10.0")]
    fn on_complete(self, result: T);
}
