use std::cmp::Ordering;
use std::fmt;

use crate::vote::RaftLeaderId;
use crate::vote::RaftVote;
use crate::vote::Vote;
use crate::vote::leader_id::raft_leader_id::RaftLeaderIdExt;
use crate::vote::ref_vote::RefVote;

/// Represents a committed Vote that has been accepted by a quorum.
///
/// The inner `Vote`'s attribute `committed` is always set to `true`
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub(crate) struct CommittedVote<LID>
where LID: RaftLeaderId
{
    leader_id: LID,
}

impl<LID> Default for CommittedVote<LID>
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

/// The `CommittedVote` is totally ordered.
///
/// Because:
/// - any two quorums have common elements,
/// - and the `CommittedVote` is accepted by a quorum,
/// - and a `Vote` is granted if it is greater than the old one.
#[allow(clippy::derive_ord_xor_partial_ord)]
impl<LID> Ord for CommittedVote<LID>
where LID: RaftLeaderId
{
    fn cmp(&self, other: &Self) -> Ordering {
        let self_ref = RefVote::new(&self.leader_id, true);
        let other_ref = RefVote::new(&other.leader_id, true);
        self_ref.partial_cmp(&other_ref).unwrap()
    }
}

impl<LID> CommittedVote<LID>
where LID: RaftLeaderId
{
    pub(crate) fn new(leader_id: LID) -> Self {
        Self { leader_id }
    }

    pub(crate) fn committed_leader_id(&self) -> LID::Committed {
        self.leader_id().to_committed()
    }

    #[allow(dead_code)]
    pub(crate) fn node_id(&self) -> &LID::NodeId {
        self.leader_id.node_id()
    }

    /// Convert to the user-facing vote type.
    pub(crate) fn into_vote<V: RaftVote<LeaderId = LID>>(self) -> V {
        V::from_leader_id(self.leader_id, true)
    }

    pub(crate) fn into_internal_vote(self) -> Vote<LID> {
        Vote::from_leader_id(self.leader_id, true)
    }
}

impl<LID> fmt::Display for CommittedVote<LID>
where LID: RaftLeaderId
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ref_vote = RefVote::new(&self.leader_id, true);
        ref_vote.fmt(f)
    }
}

impl<LID> RaftVote for CommittedVote<LID>
where LID: RaftLeaderId
{
    type LeaderId = LID;

    fn from_leader_id(_leader_id: LID, _committed: bool) -> Self {
        unreachable!("CommittedVote should only be built from a Vote")
    }

    fn leader_id(&self) -> &LID {
        &self.leader_id
    }

    fn is_committed(&self) -> bool {
        true
    }
}
