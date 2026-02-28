use crate::RaftTypeConfig;
use crate::StorageError;

/// Unrecoverable error that causes Raft to shut down.
///
/// When a `Fatal` error occurs, the Raft node stops processing requests and enters a stopped state.
/// Applications should monitor for fatal errors and initiate graceful shutdown when detected.
///
/// # Variants
///
/// - `StorageError`: Underlying storage (log or state machine) encountered an error
/// - `Panicked`: Raft core task panicked due to a programming error
/// - `Stopped`: Raft was explicitly shut down via [`Raft::shutdown`]
///
/// [`Raft::shutdown`]: crate::Raft::shutdown
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

    /// Raft stopped normally.
    #[error("raft stopped")]
    Stopped,
}
