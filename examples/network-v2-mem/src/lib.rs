//! An in-process [`RaftNetworkV2`] transport for examples and tests.
//!
//! A [`Registry`] holds the [`Raft`] handles of one cluster by node ID. Each node gets a
//! [`NetworkFactory`] from the registry, and every [`Connection`] that the factory builds sends an
//! RPC by calling a public method of the target's handle:
//!
//! | RPC               | The target calls                   |
//! |-------------------|------------------------------------|
//! | `append_entries`  | [`Raft::append_entries()`]         |
//! | `vote`            | [`Raft::vote()`]                   |
//! | `pre_vote`        | [`Raft::pre_vote()`]               |
//! | `full_snapshot`   | [`Raft::install_full_snapshot()`]  |
//! | `transfer_leader` | [`Raft::handle_transfer_leader()`] |
//!
//! Requests and responses pass by value, so nothing is serialized. A snapshot is passed as the
//! state machine's own [`SnapshotData`](openraft::storage::RaftStateMachine::SnapshotData).
//!
//! [`RaftNetworkV2`]: openraft::network::v2::RaftNetworkV2
//! [`Raft`]: openraft::Raft
//! [`Raft::append_entries()`]: openraft::Raft::append_entries
//! [`Raft::vote()`]: openraft::Raft::vote
//! [`Raft::pre_vote()`]: openraft::Raft::pre_vote
//! [`Raft::install_full_snapshot()`]: openraft::Raft::install_full_snapshot
//! [`Raft::handle_transfer_leader()`]: openraft::Raft::handle_transfer_leader

#![deny(unused_qualifications)]

mod connection;
mod network_factory;
mod registry;

pub use connection::Connection;
pub use network_factory::NetworkFactory;
pub use registry::Registry;
