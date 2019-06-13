mod raft;

pub use crate::proto::raft::{
    RaftRequest, raft_request,
    RaftResponse, raft_response,
    AppendEntriesRequest, AppendEntriesResponse,
    Entry, EntryType,
    VoteRequest, VoteResponse,
    InstallSnapshotRequest, InstallSnapshotResponse,
};
