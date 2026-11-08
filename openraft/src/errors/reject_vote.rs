use crate::RaftTypeConfig;
use crate::type_config::alias::VoteOf;

/// The local vote is greater than the received vote.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("reject vote: local vote {higher} is >= the received vote")]
pub struct RejectVote<C: RaftTypeConfig> {
    /// The vote of the rejecting node, which is greater than or equal to the received vote.
    pub higher: VoteOf<C>,
}
