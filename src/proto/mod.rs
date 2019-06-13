mod raft;

pub use crate::proto::raft::{
    RaftMessage,
    AppendEntriesRequest, AppendEntriesResponse,
    Entry, EntryType,
    VoteRequest, VoteResponse,
    InstallSnapshotRequest, InstallSnapshotResponse,
};
