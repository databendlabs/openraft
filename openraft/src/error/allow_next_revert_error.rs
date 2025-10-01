use crate::RaftTypeConfig;
use crate::error::ForwardToLeader;
use crate::error::NodeNotFound;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum AllowNextRevertError<C: RaftTypeConfig> {
    #[error("cannot set allow_next_revert; error: {0}")]
    NodeNotFound(#[from] NodeNotFound<C>),
    #[error("cannot set allow_next_revert; error: {0}")]
    ForwardToLeader(#[from] ForwardToLeader<C>),
}
