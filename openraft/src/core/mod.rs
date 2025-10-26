//! Core Raft runtime and orchestration.
//!
//! This module contains [`RaftCore`], the runtime that supports the Raft algorithm implementation
//! ([`Engine`](crate::engine::Engine)).
//!
//! ## Architecture
//!
//! [`RaftCore`] acts as the bridge between the application and the Raft consensus algorithm:
//!
//! - **Event Processing**: Receives events from application, timer, or network and passes them to
//!   [`Engine`](crate::engine::Engine)
//! - **Command Execution**: Executes [`Command`](crate::engine::Command)s emitted by
//!   [`Engine`](crate::engine::Engine) to:
//!   - Apply Raft state to underlying storage
//!   - Forward messages to other Raft nodes
//!
//! ## Key Types
//!
//! - [`RaftCore`] - Main runtime orchestrator
//! - [`ServerState`] - Server state (Leader, Follower, Candidate, Learner)
//! - [`ApplyResult`] - Result of applying log entries to state machine
//!
//! See the [Engine/Runtime architecture guide](crate::docs::components::engine_runtime) for
//! details.

pub(crate) mod balancer;
pub(crate) mod core_state;
pub(crate) mod heartbeat;
pub(crate) mod io_flush_tracking;
pub(crate) mod notification;
mod raft_core;
pub(crate) mod raft_msg;
mod replication_state;
mod server_state;
pub(crate) mod sm;
mod tick;

pub(crate) use raft_core::ApplyResult;
pub use raft_core::RaftCore;
pub(crate) use replication_state::replication_lag;
pub use server_state::ServerState;
pub(crate) use tick::Tick;
pub(crate) use tick::TickHandle;
