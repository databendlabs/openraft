use std::fmt;

use crate::RaftTypeConfig;
use crate::Vote;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::VoteOf;
use crate::vote::RaftVote;
use crate::vote::raft_vote::RaftVoteExt;

/// Represents a non-committed Vote that has **NOT** been granted by a quorum.
///
/// The inner `Vote`'s attribute `committed` is always set to `false`
#[derive(Debug, Clone, Default)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub(crate) struct NonCommittedVote<C>
where C: RaftTypeConfig
{
    leader_id: LeaderIdOf<C>,
}

impl<C> NonCommittedVote<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(leader_id: LeaderIdOf<C>) -> Self {
        Self { leader_id }
    }

    pub(crate) fn into_vote(self) -> VoteOf<C> {
        VoteOf::<C>::from_leader_id(self.leader_id, false)
    }

    pub(crate) fn into_internal_vote(self) -> Vote<C> {
        Vote::<C>::from_leader_id(self.leader_id, false)
    }
}

impl<C> RaftVote<C> for NonCommittedVote<C>
where C: RaftTypeConfig
{
    fn from_leader_id(_leader_id: C::LeaderId, _committed: bool) -> Self {
        unreachable!("NonCommittedVote should only be built from a Vote")
    }

    fn leader_id(&self) -> Option<&LeaderIdOf<C>> {
        Some(&self.leader_id)
    }

    fn is_committed(&self) -> bool {
        false
    }
}

impl<C> fmt::Display for NonCommittedVote<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref_vote().fmt(f)
    }
}
