/// All possible states of a Raft node.
#[derive(Debug, Clone, Copy, Default)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum ServerState {
    /// The node has no campaign, read voter, or write voter role in the effective membership.
    ///
    /// Uninitialized and removed nodes also use this passive state.
    #[default]
    Learner,
    /// The node is in the effective membership, is not classified as a learner, and is neither a
    /// candidate nor a leader.
    ///
    /// This state does not imply that the node has [`NodeRole::ReceiveLogs`]. For example, a read
    /// voter that is not a write voter can be a follower without receiving logs.
    ///
    /// [`NodeRole::ReceiveLogs`]: crate::membership::NodeRole::ReceiveLogs
    Follower,
    /// The node is campaigning to become the cluster leader.
    Candidate,
    /// The node is the Raft cluster leader.
    Leader,
    /// The Raft node is shutting down.
    Shutdown,
}

impl ServerState {
    /// Check if currently in learner state.
    pub fn is_learner(&self) -> bool {
        matches!(self, Self::Learner)
    }

    /// Check if currently in follower state.
    pub fn is_follower(&self) -> bool {
        matches!(self, Self::Follower)
    }

    /// Check if currently in candidate state.
    pub fn is_candidate(&self) -> bool {
        matches!(self, Self::Candidate)
    }

    /// Check if currently in leader state.
    pub fn is_leader(&self) -> bool {
        matches!(self, Self::Leader)
    }
}
