//! The `RaftCore` is a `Runtime` supporting the raft algorithm implementation `Engine`.
//!
//! It passes events from an application or timer or network to `Engine` to drive it to run.
//! Also it receives and execute `Command` emitted by `Engine` to apply raft state to underlying
//! storage or forward messages to other raft nodes.

pub(crate) mod balancer;
pub(crate) mod command_state;
pub(crate) mod notify;
mod raft_core;
pub(crate) mod raft_msg;
mod replication_state;
mod server_state;
pub(crate) mod sm;
pub(crate) mod streaming_state;
mod tick;

pub(crate) mod snapshot_state;

pub(crate) use raft_core::ApplyResult;
pub(crate) use raft_core::ApplyingEntry;
pub use raft_core::RaftCore;
pub(crate) use replication_state::replication_lag;
pub use server_state::ServerState;
pub(crate) use tick::Tick;
pub(crate) use tick::TickHandle;
