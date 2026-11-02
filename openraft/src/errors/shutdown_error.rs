use openraft_macros::since;

/// A runtime join failed for the Raft core or tick task.
///
/// The runtime error is logged when the join completes.
#[since(version = "0.10.0")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
#[error("Raft core or tick task failed to join")]
pub struct ShutdownError;
