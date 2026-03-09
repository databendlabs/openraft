use std::fmt;

use crate::vote::RaftLeaderId;
use crate::vote::RaftVote;
use crate::vote::Vote;
use crate::vote::leader_id::raft_leader_id::RaftLeaderIdExt;
use crate::vote::ref_vote::RefVote;

/// Represents a non-committed Vote that has **NOT** been granted by a quorum.
///
/// The inner `Vote`'s attribute `committed` is always set to `false`
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub(crate) struct UncommittedVote<LID>
where LID: RaftLeaderId
{
    leader_id: LID,
}

impl<LID> Default for UncommittedVote<LID>
where
    LID: RaftLeaderId,
    LID::NodeId: Default,
{
    fn default() -> Self {
        Self {
            leader_id: LID::new_with_default_term(LID::NodeId::default()),
        }
    }
}

impl<LID> UncommittedVote<LID>
where LID: RaftLeaderId
{
    pub(crate) fn new(leader_id: LID) -> Self {
        Self { leader_id }
    }

    /// Convert to the user-facing vote type.
    pub(crate) fn into_vote<V: RaftVote<LeaderId = LID>>(self) -> V {
        V::from_leader_id(self.leader_id, false)
    }

    pub(crate) fn into_internal_vote(self) -> Vote<LID> {
        Vote::from_leader_id(self.leader_id, false)
    }
}

impl<LID> RaftVote for UncommittedVote<LID>
where LID: RaftLeaderId
{
    type LeaderId = LID;

    fn from_leader_id(leader_id: LID, _committed: bool) -> Self {
        Self { leader_id }
    }

    fn leader_id(&self) -> &LID {
        &self.leader_id
    }

    fn is_committed(&self) -> bool {
        false
    }
}

impl<LID> fmt::Display for UncommittedVote<LID>
where LID: RaftLeaderId
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ref_vote = RefVote::new(&self.leader_id, false);
        ref_vote.fmt(f)
    }
}
