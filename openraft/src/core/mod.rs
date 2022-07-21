//! The `RaftCore` is a `Runtime` supporting the raft algorithm implementation `Engine`.
//!
//! It passes events from an application or timer or network to `Engine` to drive it to run.
//! Also it receives and execute `Command` emitted by `Engine` to apply raft state to underlying storage or forward
//! messages to other raft nodes.

mod install_snapshot;
mod raft_core;
pub(crate) mod replication;
mod replication_expectation;
mod replication_state;
mod server_state;
mod snapshot_state;
mod tick;

pub use raft_core::RaftCore;
pub(crate) use replication_expectation::Expectation;
pub(crate) use replication_state::replication_lag;
pub use server_state::ServerState;
pub(crate) use snapshot_state::SnapshotState;
pub(crate) use snapshot_state::SnapshotUpdate;
pub(crate) use tick::Tick;
pub(crate) use tick::VoteWiseTime;
