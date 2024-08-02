//! Raft protocol messages and types.
//!
//! Request and response types for an application to talk to the Raft,
//! and are also used by network layer to talk to other Raft nodes.

mod append_entries;
mod install_snapshot;
mod transfer_leader;
mod vote;

mod client_write;

pub use append_entries::AppendEntriesRequest;
pub use append_entries::AppendEntriesResponse;
pub use client_write::ClientWriteResponse;
pub use client_write::ClientWriteResult;
pub use install_snapshot::InstallSnapshotRequest;
pub use install_snapshot::InstallSnapshotResponse;
pub use install_snapshot::SnapshotResponse;
pub use transfer_leader::TransferLeaderRequest;
pub use vote::VoteRequest;
pub use vote::VoteResponse;
