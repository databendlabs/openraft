use crate::RaftTypeConfig;
use crate::type_config::alias::VoteOf;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("seen a higher vote: {higher} GT mine: {sender_vote}")]
pub(crate) struct HigherVote<C: RaftTypeConfig> {
    pub(crate) higher: VoteOf<C>,
    pub(crate) sender_vote: VoteOf<C>,
}
