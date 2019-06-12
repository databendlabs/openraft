mod raft;

pub use crate::proto::raft::{
    AppendEntriesRequest, AppendEntriesResponse,
    Entry, EntryType,
    VoteRequest, VoteResponse,
};
