//! Raft protocol messages and types.
//!
//! Request and response types for an application to talk to the Raft,
//! and are also used by network layer to talk to other Raft nodes.

mod append_entries_request;
mod append_entries_response;
mod change_membership;
mod install_snapshot;
mod log_segment;
mod precondition;
mod stream_append_error;
mod transfer_leader;
mod vote;
mod write;

mod client_write;
mod write_request;

pub use append_entries_request::AppendEntriesRequest;
pub use append_entries_response::AppendEntriesResponse;
pub use change_membership::ChangeMembershipOutcome;
pub use client_write::ClientWriteResponse;
pub use client_write::ClientWriteResult;
#[allow(deprecated)]
pub use install_snapshot::InstallSnapshotRequest;
#[allow(deprecated)]
pub use install_snapshot::InstallSnapshotResponse;
pub use install_snapshot::SnapshotResponse;
pub use log_segment::LogSegment;
pub use precondition::Precondition;
pub use stream_append_error::StreamAppendError;
pub use transfer_leader::TransferLeaderError;
pub use transfer_leader::TransferLeaderRequest;
pub use transfer_leader::TransferLeaderResponse;
pub use vote::VoteRequest;
pub use vote::VoteResponse;
pub use write::WriteResponse;
pub use write::WriteResult;
pub(crate) use write::into_write_result;
pub use write_request::WriteRequest;
