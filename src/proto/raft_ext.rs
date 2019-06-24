use super::raft::{
    entry, Entry, EntrySnapshotPointer,
    RaftRequest, raft_request,
    VoteRequest,
};

impl Entry {
    /// Create a new log entry which is a Snapshot pointer.
    pub fn new_snapshot_pointer(term: u64, index: u64, path: String) -> Self {
        let entry_type = Some(entry::EntryType::SnapshotPointer(EntrySnapshotPointer{path}));
        Entry{term, index, entry_type}
    }
}

impl RaftRequest {
    /// Create a new instance holding a `VoteRequest` payload.
    ///
    /// This correspond's to the Raft spec's RequestVote RPC.
    pub fn new_vote(payload: VoteRequest) -> Self {
        Self{payload: Some(raft_request::Payload::Vote(payload))}
    }
}

impl VoteRequest {
    /// Create a new instance.
    pub fn new(term: u64, candidate_id: u64, last_log_index: u64, last_log_term: u64) -> Self {
        Self{term, candidate_id, last_log_index, last_log_term}
    }
}
