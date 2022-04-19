//! The core logic of a Raft node.

mod admin;
mod append_entries;
mod candidate_state;
mod client;
mod follower_state;
mod install_snapshot;
mod leader_state;
mod learner_state;
mod raft_core;
pub(crate) mod replication;
mod replication_state;
#[cfg(test)]
mod replication_state_test;
mod server_state;
mod snapshot_state;
mod vote;

use candidate_state::CandidateState;
use follower_state::FollowerState;
use leader_state::LeaderState;
use learner_state::LearnerState;
use raft_core::apply_to_state_machine;
use raft_core::purge_applied_logs;
use raft_core::MetricsProvider;
pub use raft_core::RaftCore;
pub use replication_state::is_matched_upto_date;
use replication_state::ReplicationState;
pub use server_state::ServerState;
use snapshot_state::SnapshotState;
use snapshot_state::SnapshotUpdate;
