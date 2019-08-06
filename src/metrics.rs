use actix::prelude::*;

use crate::{
    NodeId,
    messages::MembershipConfig,
};

/// All possible states of a Raft node.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// The node is completely passive; replicating entries, but not voting or timing out.
    NonVoter,
    /// The node is actively replicating logs from the leader.
    Follower,
    /// The node has detected an election timeout so is requesting votes to become leader.
    Candidate,
    /// The node is actively functioning as the Raft cluster leader.
    Leader,
}

/// Baseline metrics of the current state of the subject Raft node.
#[derive(Clone, Debug, Message)]
pub struct RaftMetrics {
    pub id: NodeId,
    pub state: State,
    pub current_term: u64,
    pub last_log_index: u64,
    pub last_applied: u64,
    pub current_leader: Option<NodeId>,
    pub membership_config: MembershipConfig,
}
