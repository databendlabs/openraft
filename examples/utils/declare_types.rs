// Reference the containing module's type config.
use super::TypeConfig;

pub type Raft = openraft::Raft<TypeConfig>;

pub type Vote = openraft::Vote<TypeConfig>;
pub type LeaderId = openraft::LeaderId<TypeConfig>;
pub type LogId = openraft::LogId<TypeConfig>;
pub type Entry = openraft::Entry<TypeConfig>;
pub type EntryPayload = openraft::EntryPayload<TypeConfig>;
pub type StoredMembership = openraft::StoredMembership<TypeConfig>;

pub type LogState = openraft::storage::LogState<TypeConfig>;

pub type SnapshotMeta = openraft::SnapshotMeta<TypeConfig>;
pub type Snapshot = openraft::Snapshot<TypeConfig>;
pub type SnapshotData = <TypeConfig as openraft::RaftTypeConfig>::SnapshotData;

pub type IOFlushed = openraft::storage::IOFlushed<TypeConfig>;

pub type Infallible = openraft::error::Infallible;
pub type Fatal = openraft::error::Fatal<TypeConfig>;
pub type RaftError<E = openraft::error::Infallible> = openraft::error::RaftError<TypeConfig, E>;
pub type RPCError<E = openraft::error::Infallible> = openraft::error::RPCError<TypeConfig, E>;

pub type ErrorSubject = openraft::ErrorSubject<TypeConfig>;
pub type StorageError = openraft::StorageError<TypeConfig>;
pub type StreamingError = openraft::error::StreamingError<TypeConfig>;

pub type RaftMetrics = openraft::RaftMetrics<TypeConfig>;

pub type ClientWriteError = openraft::error::ClientWriteError<TypeConfig>;
pub type CheckIsLeaderError = openraft::error::CheckIsLeaderError<TypeConfig>;
pub type ForwardToLeader = openraft::error::ForwardToLeader<TypeConfig>;
pub type InitializeError = openraft::error::InitializeError<TypeConfig>;

pub type VoteRequest = openraft::raft::VoteRequest<TypeConfig>;
pub type VoteResponse = openraft::raft::VoteResponse<TypeConfig>;
pub type AppendEntriesRequest = openraft::raft::AppendEntriesRequest<TypeConfig>;
pub type AppendEntriesResponse = openraft::raft::AppendEntriesResponse<TypeConfig>;
pub type InstallSnapshotRequest = openraft::raft::InstallSnapshotRequest<TypeConfig>;
pub type InstallSnapshotResponse = openraft::raft::InstallSnapshotResponse<TypeConfig>;
pub type SnapshotResponse = openraft::raft::SnapshotResponse<TypeConfig>;
pub type ClientWriteResponse = openraft::raft::ClientWriteResponse<TypeConfig>;
