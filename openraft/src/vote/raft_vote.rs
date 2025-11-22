use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::base::OptionalFeatures;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::vote::RaftLeaderId;
use crate::vote::committed::CommittedVote;
use crate::vote::leader_id::raft_leader_id::RaftLeaderIdExt;
use crate::vote::non_committed::UncommittedVote;
use crate::vote::ref_vote::RefVote;
use crate::vote::vote_status::VoteStatus;
// TODO: OptionSerde can be removed after all types are made trait based.

/// Represents a vote in Raft consensus, including both votes for leader candidates
/// and committed leader (a leader granted by a quorum).
#[since(version = "0.10.0")]
pub trait RaftVote<C>
where
    C: RaftTypeConfig,
    Self: OptionalFeatures + Eq + Clone + Debug + Display + 'static,
{
    /// Create a new vote for the specified leader with optional quorum commitment.
    #[since(version = "0.10.0")]
    fn from_leader_id(leader_id: C::LeaderId, committed: bool) -> Self;

    /// Get a reference to this vote's LeaderId([`RaftLeaderId`] implementation).
    #[since(version = "0.10.0", change = "non-Option return")]
    #[since(version = "0.10.0")]
    fn leader_id(&self) -> &C::LeaderId;

    /// Whether this vote has been committed by a quorum.
    #[since(version = "0.10.0")]
    fn is_committed(&self) -> bool;
}

pub(crate) trait RaftVoteExt<C>
where
    C: RaftTypeConfig,
    Self: RaftVote<C>,
{
    #[allow(dead_code)]
    fn new_with_default_term(node_id: C::NodeId) -> Self {
        let leader_id = C::LeaderId::new_with_default_term(node_id);
        Self::from_leader_id(leader_id, false)
    }

    /// Creates a new vote for a node in a specific term, with uncommitted status.
    fn from_term_node_id(term: C::Term, node_id: C::NodeId) -> Self {
        let leader_id = C::LeaderId::new(term, node_id);
        Self::from_leader_id(leader_id, false)
    }

    fn from_term_node_id_committed(term: C::Term, node_id: C::NodeId, committed: bool) -> Self {
        let leader_id = C::LeaderId::new(term, node_id);
        Self::from_leader_id(leader_id, committed)
    }

    fn term(&self) -> C::Term {
        self.leader_id().term()
    }

    /// Gets the node ID of the leader this vote is for.
    fn to_leader_node_id(&self) -> C::NodeId {
        self.leader_node_id().clone()
    }

    /// Gets a reference to the node ID of the leader this vote is for.
    fn leader_node_id(&self) -> &C::NodeId {
        self.leader_id().node_id()
    }

    /// Gets the leader ID this vote is associated with.
    fn to_leader_id(&self) -> C::LeaderId {
        self.leader_id().clone()
    }

    /// Creates a reference view of this vote.
    ///
    /// Returns a lightweight `RefVote` that borrows the data from this vote.
    fn as_ref_vote(&self) -> RefVote<'_, C> {
        RefVote::new(self.leader_id(), self.is_committed())
    }

    /// Create a [`CommittedVote`] with the same leader id.
    fn to_committed(&self) -> CommittedVote<C> {
        CommittedVote::new(self.to_leader_id())
    }

    /// Create a [`UncommittedVote`] with the same leader id.
    fn to_non_committed(&self) -> UncommittedVote<C> {
        UncommittedVote::new(self.to_leader_id())
    }

    /// Convert this vote into a [`CommittedVote`]
    fn into_committed(self) -> CommittedVote<C> {
        CommittedVote::new(self.to_leader_id())
    }

    /// Convert this vote into a [`UncommittedVote`]
    fn into_non_committed(self) -> UncommittedVote<C> {
        UncommittedVote::new(self.to_leader_id())
    }

    /// Converts this vote into a [`VoteStatus`] enum based on its commitment state.
    fn into_vote_status(self) -> VoteStatus<C> {
        if self.is_committed() {
            VoteStatus::Committed(self.into_committed())
        } else {
            VoteStatus::Pending(self.into_non_committed())
        }
    }

    /// Converts this vote to a [`CommittedVote`] if it is committed.
    ///
    /// Returns `Some(CommittedVote)` if the vote is committed, otherwise returns `None`.
    #[allow(dead_code)]
    fn try_to_committed(&self) -> Option<CommittedVote<C>> {
        if self.is_committed() {
            Some(self.to_committed())
        } else {
            None
        }
    }

    /// Returns the leader ID as `CommittedLeaderId` if this vote is committed.
    ///
    /// Returns `Some(CommittedLeaderId)` if the vote is committed and has a leader ID.
    /// Returns `None` if the vote is not committed or has no leader ID.
    #[allow(dead_code)]
    fn try_to_committed_leader_id(&self) -> Option<CommittedLeaderIdOf<C>> {
        if self.is_committed() {
            Some(self.leader_id().to_committed())
        } else {
            None
        }
    }

    /// Checks if this vote is for the same leader as specified by the given committed leader ID.
    fn is_same_leader(&self, leader_id: &CommittedLeaderIdOf<C>) -> bool {
        self.leader_id().to_committed() == *leader_id
    }
}

impl<C, T> RaftVoteExt<C> for T
where
    C: RaftTypeConfig,
    T: RaftVote<C>,
{
}
