//! The Raft network interface.
//!
//! This module defines traits for implementing network communication between Raft nodes:
//!
//! ## Network Traits
//!
//! - [`RaftNetwork`] - Protocol for sending Raft RPCs (AppendEntries, Vote, InstallSnapshot)
//! - [`RaftNetworkFactory`] - Factory for creating network connections to target nodes
//! - [`v2::RaftNetworkV2`] - Alternative protocol with full snapshot support
//!
//! ## Key Types
//!
//! - [`Backoff`] - Backoff strategy for retrying failed network operations
//! - [`RPCOption`] - Options for configuring RPC behavior
//! - [`RPCTypes`] - Type definitions for RPC requests and responses
//!
//! ## Usage
//!
//! Applications implement [`RaftNetworkFactory`] to create [`RaftNetwork`] instances
//! for communicating with each remote Raft node. The factory is passed to
//! [`Raft::new()`](crate::Raft::new) when creating a Raft instance.
//!
//! See the [Getting Started Guide](crate::docs::getting_started) for implementation
//! details and examples.

mod backoff;
mod rpc_option;
mod rpc_type;

pub mod v1;
pub mod v2;

pub mod snapshot_transport;

pub use backoff::Backoff;
pub use rpc_option::RPCOption;
pub use rpc_type::RPCTypes;
pub use v1::RaftNetwork;
pub use v1::RaftNetworkFactory;
