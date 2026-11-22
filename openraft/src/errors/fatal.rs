use crate::RaftTypeConfig;
use crate::StorageError;

/// Unrecoverable error that stops the Raft node.
///
/// `RaftCore`, the task that runs the protocol, stops with a `Fatal` error. API calls on the
/// stopped node return that error, and [`RaftMetrics::running_state`] holds it. An application
/// that gets a `Fatal` error should take the node out of service and shut it down with
/// [`Raft::shutdown`]. Getting the error does not mean that the node's tasks have exited, and a
/// shutdown joins only the `RaftCore` task.
///
/// # Variants
///
/// - `StorageError`: Underlying storage (log or state machine) encountered an error
/// - `Panicked`: Raft core task panicked due to a programming error
/// - `Stopped`: Raft core task stopped for another reason, such as a shutdown or the drop of every
///   `Raft` handle
///
/// [`Raft::shutdown`]: crate::Raft::shutdown
/// [`RaftMetrics::running_state`]: crate::RaftMetrics::running_state
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum Fatal<C>
where C: RaftTypeConfig
{
    /// Storage error that caused the Raft node to stop.
    #[error(transparent)]
    StorageError(#[from] StorageError<C>),

    /// Raft node panicked and stopped.
    #[error("panicked")]
    Panicked,

    /// Raft core task stopped without a storage error or a panic.
    ///
    /// The core task stops this way when its control channel closes, by [`Raft::shutdown`] or by
    /// dropping every `Raft` handle. A node shut down before [`Raft::run`] reports `Stopped` too,
    /// and so does a core task whose runtime join fails without a panic, such as by cancellation.
    ///
    /// So `Stopped` alone does not tell an explicit shutdown from a dropped handle, and it does
    /// not mean that the core task joined successfully: [`Raft::try_shutdown`] returns an error
    /// when that join failed.
    ///
    /// [`Raft::run`]: crate::Raft::run
    /// [`Raft::shutdown`]: crate::Raft::shutdown
    /// [`Raft::try_shutdown`]: crate::Raft::try_shutdown
    #[error("raft stopped")]
    Stopped,
}
