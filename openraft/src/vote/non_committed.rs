use std::fmt;

use crate::RaftTypeConfig;
use crate::Vote;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::VoteOf;
use crate::vote::RaftVote;
use crate::vote::leader_id::raft_leader_id::RaftLeaderIdExt;
use crate::vote::raft_vote::RaftVoteExt;

/// Represents a non-committed Vote that has **NOT** been granted by a quorum.
///
/// The inner `Vote`'s attribute `committed` is always set to `false`
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub(crate) struct UncommittedVote<C>
where C: RaftTypeConfig
{
    leader_id: LeaderIdOf<C>,
}

impl<C> Default for UncommittedVote<C>
where
    C: RaftTypeConfig,
    C::NodeId: Default,
{
    fn default() -> Self {
        Self {
            leader_id: LeaderIdOf::<C>::new_with_default_term(C::NodeId::default()),
        }
    }
}

impl<C> UncommittedVote<C>
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

impl<C> RaftVote<C> for UncommittedVote<C>
where C: RaftTypeConfig
{
    fn from_leader_id(_leader_id: C::LeaderId, _committed: bool) -> Self {
        unreachable!("NonCommittedVote should only be built from a Vote")
    }

    fn leader_id(&self) -> &LeaderIdOf<C> {
        &self.leader_id
    }

    fn is_committed(&self) -> bool {
        false
    }
}

impl<C> fmt::Display for UncommittedVote<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref_vote().fmt(f)
    }
}
