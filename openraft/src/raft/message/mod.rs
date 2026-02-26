//! Raft protocol messages and types.
//!
//! Request and response types for an application to talk to the Raft,
//! and are also used by network layer to talk to other Raft nodes.

mod append_entries;
mod append_entries_request;
mod append_entries_response;
mod install_snapshot;
mod log_segment;
mod matched_log_id;
mod stream_append_error;
mod transfer_leader;
mod vote;
mod write;

mod client_write;
mod write_request;

pub use append_entries::AppendEntries;
pub use append_entries_request::AppendEntriesRequest;
pub use append_entries_response::AppendEntriesResponse;
pub use client_write::ClientWriteResponse;
pub use client_write::ClientWriteResult;
pub use install_snapshot::InstallSnapshotRequest;
pub use install_snapshot::InstallSnapshotResponse;
pub use install_snapshot::SnapshotResponse;
pub use log_segment::LogSegment;
pub use matched_log_id::MatchedLogId;
pub use stream_append_error::StreamAppendError;
pub use transfer_leader::TransferLeaderRequest;
pub use vote::VoteRequest;
pub use vote::VoteResponse;
pub use write::WriteResponse;
pub use write::WriteResult;
pub(crate) use write::into_write_result;
pub use write_request::WriteRequest;

use crate::errors::RejectAppendEntries;

/// Result of an AppendEntries operation: the matched log id on success, or a rejection error.
pub type AppendEntriesResult<C> = Result<MatchedLogId<C>, RejectAppendEntries<C>>;
