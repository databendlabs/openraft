use crate::RaftTypeConfig;
use crate::TryAsRef;
use crate::error::ForwardToLeader;
use crate::error::QuorumNotEnough;

/// An error related to an is_leader request.
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum LinearizableReadError<C>
where C: RaftTypeConfig
{
    /// This node is not the leader; request should be forwarded to the leader.
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<C>),

    /// Cannot finish a request, such as elect or replicate, because a quorum is not available.
    #[error(transparent)]
    QuorumNotEnough(#[from] QuorumNotEnough<C>),
}

impl<C> TryAsRef<ForwardToLeader<C>> for LinearizableReadError<C>
where C: RaftTypeConfig
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<C>> {
        match self {
            Self::ForwardToLeader(f) => Some(f),
            _ => None,
        }
    }
}
