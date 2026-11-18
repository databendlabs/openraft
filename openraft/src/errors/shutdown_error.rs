use openraft_macros::since;

/// A runtime join failed for the Raft core task.
///
/// The runtime error is logged when the join completes.
#[since(
    version = "0.10.1",
    change = "only the core task is joined: the tick task is retired"
)]
#[since(version = "0.10.0")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
#[error("Raft core task failed to join")]
pub struct ShutdownError;
