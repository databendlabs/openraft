//! Multi-Raft support for managing multiple Raft groups on a single node.
//!
//! This module provides types and traits for running multiple independent Raft groups
//! within a single process.
//!
//! ## Key Components
//!
//! - [`GroupId`] - Trait for Raft group identifiers
//! - [`MultiRaftTypeConfig`] - Extension of [`RaftTypeConfig`] with group support
//!
//! ## Overview
//!
//! In a Multi-Raft setup, a single physical node can host replicas from multiple Raft groups.
//! Each group operates independently with its own:
//! - Log storage
//! - State machine
//! - Membership configuration
//! - Leader election
//!
//! The [`GroupId`] uniquely identifies each Raft group, allowing RPC handlers to route
//! requests to the correct Raft instance.
//!
//! ## Example
//!
//! ```ignore
//! use openraft::multi_raft::{GroupId, MultiRaftTypeConfig};
//!
//! // Define your type config with GroupId support
//! openraft::declare_raft_types!(
//!     pub MyConfig:
//!         D = MyRequest,
//!         R = MyResponse,
//! );
//!
//! impl MultiRaftTypeConfig for MyConfig {
//!     type GroupId = String;  // or u64, custom type, etc.
//! }
//! ```
//!
//! [`RaftTypeConfig`]: crate::RaftTypeConfig

mod group_id;
mod network;
mod router;
mod rpc;
mod type_config;

pub use group_id::GroupId;
pub use network::GroupNetworkAdapter;
pub use network::MultiRaftNetwork;
pub use network::MultiRaftNetworkFactory;
pub use router::RaftRouter;
pub use rpc::GroupedAppendEntriesRequest;
pub use rpc::GroupedAppendEntriesResponse;
pub use rpc::GroupedInstallSnapshotRequest;
pub use rpc::GroupedInstallSnapshotResponse;
pub use rpc::GroupedRequest;
pub use rpc::GroupedResponse;
pub use rpc::GroupedSnapshotResponse;
pub use rpc::GroupedVoteRequest;
pub use rpc::GroupedVoteResponse;
pub use type_config::MultiRaftTypeConfig;

pub use crate::error::MultiRaftError;

#[cfg(test)]
mod tests;
