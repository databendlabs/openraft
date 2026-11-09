use openraft_macros::since;

use crate::metrics::WaitError;

/// Errors returned when waiting for a leader's replication to catch up.
#[since(version = "0.10.0")]
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum ReplicationWaitError {
    /// The metrics wait timed out or the Raft node shut down.
    #[error(transparent)]
    Wait(#[from] WaitError),

    /// The local node is not a leader, so its replication progress is unavailable.
    #[error("the local node is not a leader")]
    NotLeader,

    /// The target node is absent from the effective membership.
    #[error("the replication target is not in the effective membership")]
    NodeNotFound,
}
