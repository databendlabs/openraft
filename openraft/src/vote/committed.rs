use std::cmp::Ordering;
use std::fmt;

use crate::RaftTypeConfig;
use crate::Vote;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::VoteOf;
use crate::vote::RaftLeaderId;
use crate::vote::RaftVote;
use crate::vote::raft_vote::RaftVoteExt;

/// Represents a committed Vote that has been accepted by a quorum.
///
/// The inner `Vote`'s attribute `committed` is always set to `true`
#[derive(Debug, Clone, Default)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub(crate) struct CommittedVote<C>
where C: RaftTypeConfig
{
    leader_id: LeaderIdOf<C>,
}

/// The `CommittedVote` is totally ordered.
///
/// Because:
/// - any two quorums have common elements,
/// - and the `CommittedVote` is accepted by a quorum,
/// - and a `Vote` is granted if it is greater than the old one.
#[allow(clippy::derive_ord_xor_partial_ord)]
impl<C> Ord for CommittedVote<C>
where C: RaftTypeConfig
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref_vote().partial_cmp(&other.as_ref_vote()).unwrap()
    }
}

impl<C> CommittedVote<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(leader_id: LeaderIdOf<C>) -> Self {
        Self { leader_id }
    }

    pub(crate) fn committed_leader_id(&self) -> CommittedLeaderIdOf<C> {
        self.leader_id().map_or_else(Default::default, RaftLeaderId::to_committed)
    }

    pub(crate) fn into_vote(self) -> VoteOf<C> {
        VoteOf::<C>::from_leader_id(self.leader_id, true)
    }

    pub(crate) fn into_internal_vote(self) -> Vote<C> {
        Vote::<C>::from_leader_id(self.leader_id, true)
    }
}

impl<C> fmt::Display for CommittedVote<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref_vote().fmt(f)
    }
}

impl<C> RaftVote<C> for CommittedVote<C>
where C: RaftTypeConfig
{
    fn from_leader_id(_leader_id: C::LeaderId, _committed: bool) -> Self {
        unreachable!("CommittedVote should only be built from a Vote")
    }

    fn leader_id(&self) -> Option<&LeaderIdOf<C>> {
        Some(&self.leader_id)
    }

    fn is_committed(&self) -> bool {
        true
    }
}
