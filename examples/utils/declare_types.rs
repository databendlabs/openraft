//! Declare the Raft type with the TypeConfig.

// Reference the containing module's type config and re-export it.
// Re-export `Raft` from the parent module so `typ::Raft` works.
pub use super::Raft;
pub use super::TypeConfig;

pub type Vote = <TypeConfig as openraft::RaftTypeConfig>::Vote;
pub type LeaderId = <TypeConfig as openraft::RaftTypeConfig>::LeaderId;
pub type LogId = openraft::alias::LogIdOf<TypeConfig>;
pub type Entry = <TypeConfig as openraft::RaftTypeConfig>::Entry;
pub type EntryPayload = openraft::alias::EntryPayloadOf<TypeConfig>;
pub type Membership = openraft::membership::Membership<
    <TypeConfig as openraft::RaftTypeConfig>::NodeId,
    <TypeConfig as openraft::RaftTypeConfig>::Node,
>;
pub type StoredMembership = openraft::alias::StoredMembershipOf<TypeConfig>;

pub type ApplyResponder = openraft::storage::ApplyResponder<TypeConfig>;
pub type EntryResponder = openraft::storage::EntryResponder<TypeConfig>;

pub type Node = <TypeConfig as openraft::RaftTypeConfig>::Node;

pub type LogState = openraft::storage::LogState<TypeConfig>;

pub type SnapshotMeta = openraft::alias::SnapshotMetaOf<TypeConfig>;
pub type Snapshot = openraft::alias::SnapshotOf<TypeConfig>;
pub type SnapshotData = <TypeConfig as openraft::RaftTypeConfig>::SnapshotData;

pub type IOFlushed = openraft::storage::IOFlushed<TypeConfig>;

pub type Infallible = openraft::errors::Infallible;
pub type Fatal = openraft::errors::Fatal<TypeConfig>;
pub type RaftError<E = openraft::errors::Infallible> = openraft::errors::RaftError<TypeConfig, E>;
pub type RPCError<E = openraft::errors::Infallible> = openraft::errors::RPCError<TypeConfig, E>;

pub type ErrorSubject = openraft::ErrorSubject<TypeConfig>;
pub type StorageError = openraft::StorageError<TypeConfig>;
pub type StreamingError = openraft::errors::StreamingError<TypeConfig>;

pub type RaftMetrics = openraft::RaftMetrics<TypeConfig>;

pub type ClientWriteError = openraft::errors::ClientWriteError<TypeConfig>;
pub type LinearizableReadError = openraft::errors::LinearizableReadError<TypeConfig>;
pub type ForwardToLeader = openraft::errors::ForwardToLeader<TypeConfig>;
pub type InitializeError = openraft::errors::InitializeError<TypeConfig>;

pub type VoteRequest = openraft::raft::VoteRequest<TypeConfig>;
pub type VoteResponse = openraft::raft::VoteResponse<TypeConfig>;
pub type AppendEntriesRequest = openraft::raft::AppendEntriesRequest<TypeConfig>;
pub type AppendEntriesResponse = openraft::raft::AppendEntriesResponse<TypeConfig>;
pub type InstallSnapshotRequest = openraft::raft::InstallSnapshotRequest<TypeConfig>;
pub type InstallSnapshotResponse = openraft::raft::InstallSnapshotResponse<TypeConfig>;
pub type SnapshotResponse = openraft::raft::SnapshotResponse<TypeConfig>;
pub type ClientWriteResponse = openraft::raft::ClientWriteResponse<TypeConfig>;
pub type StreamAppendResult = openraft::raft::StreamAppendResult<TypeConfig>;
