use std::fmt::Debug;
use std::fmt::Display;

use crate::alias::CommittedLeaderIdOf;
use crate::base::cmp::CompareByKey;
use crate::base::OptionalFeatures;
use crate::vote::committed::CommittedVote;
use crate::vote::non_committed::NonCommittedVote;
use crate::vote::ref_vote::RefVote;
use crate::vote::vote_status::VoteStatus;
use crate::vote::RaftLeaderId;
use crate::RaftTypeConfig;
// TODO: OptionSerde can be removed after all types are made trait based.

/// Represents a vote in Raft consensus, including both votes for leader candidates
/// and committed leader(a leader granted by a quorum).
pub trait RaftVote<C>
where
    C: RaftTypeConfig,
    Self: OptionalFeatures + Eq + Clone + Debug + Display + Default + 'static,
{
    /// Create a new vote for the specified leader with optional quorum commitment
    fn from_leader_id(leader_id: C::LeaderId, committed: bool) -> Self;

    /// Get a reference to this vote's LeaderId([`RaftLeaderId`] implementation)
    fn leader_id_ref(&self) -> &C::LeaderId;

    /// Whether this vote has been committed by a quorum
    fn is_committed(&self) -> bool;
}

pub(crate) trait RaftVoteExt<C>
where
    C: RaftTypeConfig,
    Self: RaftVote<C>,
{
    /// Creates a new vote for a node in a specific term, with uncommitted status.
    fn from_term_node_id(term: C::Term, node_id: C::NodeId) -> Self {
        let leader_id = C::LeaderId::new(term, node_id);
        Self::from_leader_id(leader_id, false)
    }

    /// Gets the node ID of the leader this vote is for, if present.
    fn leader_node_id(&self) -> Option<C::NodeId> {
        self.leader_id_ref().node_id_ref().cloned()
    }

    /// Gets a reference to the node ID of the leader this vote is for, if present.

    fn leader_node_id_ref(&self) -> Option<&C::NodeId> {
        self.leader_id_ref().node_id_ref()
    }
    /// Gets the leader ID this vote is associated with.

    fn leader_id(&self) -> C::LeaderId {
        self.leader_id_ref().clone()
    }

    /// Creates a reference view of this vote.
    ///
    /// Returns a lightweight `RefVote` that borrows the data from this vote.
    fn as_ref_vote(&self) -> RefVote<'_, C> {
        RefVote::new(self.leader_id_ref(), self.is_committed())
    }

    /// Create a [`CommittedVote`] with the same leader id.
    fn to_committed(&self) -> CommittedVote<C> {
        CommittedVote::new(self.leader_id())
    }

    /// Create a [`NonCommittedVote`] with the same leader id.
    fn to_non_committed(&self) -> NonCommittedVote<C> {
        NonCommittedVote::new(self.leader_id())
    }

    /// Convert this vote into a [`CommittedVote`]
    fn into_committed(self) -> CommittedVote<C> {
        CommittedVote::new(self.leader_id())
    }

    /// Convert this vote into a [`NonCommittedVote`]
    fn into_non_committed(self) -> NonCommittedVote<C> {
        NonCommittedVote::new(self.leader_id())
    }

    /// Converts this vote into a [`VoteStatus`] enum based on its commitment state.
    fn into_vote_status(self) -> VoteStatus<C> {
        if self.is_committed() {
            VoteStatus::Committed(self.into_committed())
        } else {
            VoteStatus::Pending(self.into_non_committed())
        }
    }

    /// Checks if this vote is for the same leader as specified by the given committed leader ID.
    fn is_same_leader(&self, leader_id: &CommittedLeaderIdOf<C>) -> bool {
        self.leader_id_ref().to_committed() == *leader_id
    }
}

impl<C, T> RaftVoteExt<C> for T
where
    C: RaftTypeConfig,
    T: RaftVote<C>,
{
}

impl<C, T> CompareByKey<C> for T
where
    C: RaftTypeConfig,
    T: RaftVote<C>,
{
    type Key<'k> = RefVote<'k, C> where Self: 'k;

    fn cmp_key(&self) -> Self::Key<'_> {
        RefVote::new(self.leader_id_ref(), self.is_committed())
    }
}
