//! Raft metrics for observability.
//!
//! The `RaftMetrics` type derives the `actix::Message` type, so applications are expected to
//! implement a handler for receiving these metrics, and then should pass its
//! `actix::Receiver<RaftMetrics>` to the `Raft` instance constructor when starting a Raft node.
//!
//! The `RaftMetrics` type holds the baseline metrics on the state of the Raft node the metrics
//! are coming from, its current role in the cluster, its current membership config, as well as
//! information on the Raft log and the last index to be applied to the state machine.
//!
//! Applications may use this data in whatever way is needed. The obvious use cases are to expose
//! these metrics to a metrics collection system like Prometheus or Influx. Applications may also
//! use this data to trigger events within higher levels of the parent application.
//!
//! Metrics will be exported at a regular interval according to the
//! [Config.metrics_rate](https://docs.rs/actix-raft/latest/actix-raft/config/struct.Config.html#structfield.metrics_rate)
//! value, but will also emit a new metrics record any time the `state` of the Raft node changes,
//! the `membership_config` changes, or the `current_leader` changes.

use actix::prelude::*;

use crate::{
    NodeId,
    messages::MembershipConfig,
};

/// All possible states of a Raft node.
#[derive(Clone, Debug, PartialEq, Eq)]
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
///
/// See the [module level documentation](https://docs.rs/actix-raft/latest/actix-raft/metrics/index.html)
/// for more details.
#[derive(Clone, Debug, Message, PartialEq, Eq)]
pub struct RaftMetrics {
    /// The ID of the Raft node.
    pub id: NodeId,
    /// The state of the Raft node.
    pub state: State,
    /// The current term of the Raft node.
    pub current_term: u64,
    /// The last log index to be appended to this Raft node's log.
    pub last_log_index: u64,
    /// The last log index to be applied to this Raft node's state machine.
    pub last_applied: u64,
    /// The current cluster leader.
    pub current_leader: Option<NodeId>,
    /// The current membership config of the cluster.
    pub membership_config: MembershipConfig,
}
