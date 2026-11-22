use crate::vote::RaftVote;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("seen a higher vote: {higher} GT mine: {sender_vote}")]
pub(crate) struct HigherVote<V>
where V: RaftVote
{
    pub(crate) higher: V,
    pub(crate) sender_vote: V,
}
