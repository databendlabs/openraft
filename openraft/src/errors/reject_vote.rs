use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::VoteOf;

/// The received vote is neither equal to nor greater than the local vote.
///
/// The received vote is smaller than the local vote, or it is not comparable with it: with
/// standard leader IDs, two uncommitted votes of one term for different nodes are not comparable.
#[since(
    version = "0.10.2",
    change = "the message no longer says the local vote is >= the received vote"
)]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("reject vote: rejected by local vote {higher}")]
pub struct RejectVote<C: RaftTypeConfig> {
    /// The local vote of the rejecting node.
    pub higher: VoteOf<C>,
}
