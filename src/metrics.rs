use actix::prelude::*;

use crate::{
    NodeId,
};

/// All possible states of a Raft node.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// A non-standard Raft state indicating that the node is awaiting an admin command to begin.
    Standby,
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
}
