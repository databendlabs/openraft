use crate::RaftTypeConfig;
use crate::error::ForwardToLeader;
use crate::error::NodeNotFound;

/// Error related to setting the allow_next_revert flag.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum AllowNextRevertError<C: RaftTypeConfig> {
    /// The target node was not found.
    #[error("cannot set allow_next_revert; error: {0}")]
    NodeNotFound(#[from] NodeNotFound<C>),
    /// Request must be forwarded to the leader.
    #[error("cannot set allow_next_revert; error: {0}")]
    ForwardToLeader(#[from] ForwardToLeader<C>),
}
