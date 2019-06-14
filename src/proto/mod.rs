mod raft;
mod raft_ext;

pub use crate::proto::raft::{
    RaftRequest, raft_request,
    RaftResponse, raft_response,
    AppendEntriesRequest, AppendEntriesResponse, ConflictOpt,
    Entry, entry, EntryNormal, EntryConfigChange, EntrySnapshotPointer,
    VoteRequest, VoteResponse,
    InstallSnapshotRequest, InstallSnapshotResponse,
};
