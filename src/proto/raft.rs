/// A Raft message containing a specific RPC payload.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RaftMessage {
    #[prost(oneof="raft_message::Payload", tags="1, 2, 3, 4, 5, 6")]
    pub payload: ::std::option::Option<raft_message::Payload>,
}
pub mod raft_message {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Payload {
        #[prost(message, tag="1")]
        AppendEntriesRequest(super::AppendEntriesRequest),
        #[prost(message, tag="2")]
        AppendEntriesResponse(super::AppendEntriesResponse),
        #[prost(message, tag="3")]
        VoteRequest(super::VoteRequest),
        #[prost(message, tag="4")]
        VoteResponse(super::VoteResponse),
        #[prost(message, tag="5")]
        InstallSnapshotRequest(super::InstallSnapshotRequest),
        #[prost(message, tag="6")]
        InstallSnapshotResponse(super::InstallSnapshotResponse),
    }
}
/// An RPC invoked by the leader to replicate log entries (§5.3); also used as heartbeat (§5.2).
///
/// Receiver implementation:
///
/// 1. Reply `false` if `term` is less than node's current `term` (§5.1).
/// 2. Reply `false` if log doesn’t contain an entry at `prev_log_index` whose term
///    matches `prev_log_term` (§5.3).
/// 3. If an existing entry conflicts with a new one (same index but different terms), delete the
///    existing entry and all thatfollow it (§5.3).
/// 4. Append any new entries not already in the log.
/// 5. If `leader_commit` is greater than node's commit index, set nodes commit index to
///    `min(leader_commit, index of last new entry)`.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AppendEntriesRequest {
    /// The leader's current term.
    #[prost(uint64, required, tag="1")]
    pub term: u64,
    /// The leader's ID. Useful in redirecting clients.
    #[prost(uint64, required, tag="2")]
    pub leader_id: u64,
    /// The index of the log entry immediately preceding the new entries.
    #[prost(uint64, required, tag="3")]
    pub prev_log_index: u64,
    /// The term of the `prev_log_index` entry.
    #[prost(uint64, required, tag="4")]
    pub prev_log_term: u64,
    /// The new log entries to store.
    ///
    /// This may be empty when the leader is sending heartbeats. Entries
    /// may be batched for efficiency.
    #[prost(message, repeated, tag="5")]
    pub entries: ::std::vec::Vec<Entry>,
    /// The leader's commit index.
    #[prost(uint64, required, tag="6")]
    pub leader_commit: u64,
}
/// An RPC response to an `AppendEntriesRequest` message.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AppendEntriesResponse {
    /// The responding node's current term, for leader to update itself.
    #[prost(uint64, required, tag="1")]
    pub term: u64,
    /// Will be true if follower contained entry matching `prev_log_index` and `prev_log_term`.
    #[prost(bool, required, tag="2")]
    pub success: bool,
}
/// An entry to be committed to the Raft log.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Entry {
    /// This entry's type.
    #[prost(enumeration="EntryType", required, tag="1")]
    pub entry_type: i32,
    #[prost(uint64, required, tag="2")]
    pub term: u64,
    #[prost(uint64, required, tag="3")]
    pub index: u64,
    #[prost(bytes, required, tag="4")]
    pub data: std::vec::Vec<u8>,
}
/// An RPC invoked by candidates to gather votes (§5.2).
///
/// Receiver implementation:
///
/// 1. Reply `false` if `term` is less than receiver's current `term` (§5.1).
/// 2. If receiver has not cast a vote for the current `term` or it voted for `candidate_id`, and
///    candidate’s log is atleast as up-to-date as receiver’s log, grant vote (§5.2, §5.4).
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct VoteRequest {
    /// The candidate's current term.
    #[prost(uint64, required, tag="1")]
    pub term: u64,
    /// The candidate's ID.
    #[prost(uint64, required, tag="2")]
    pub candidate_id: u64,
    /// The index of the candidate’s last log entry (§5.4).
    #[prost(uint64, required, tag="3")]
    pub last_log_index: u64,
    /// The term of the candidate’s last log entry (§5.4).
    #[prost(uint64, required, tag="4")]
    pub last_log_term: u64,
}
/// An RPC response to an `VoteResponse` message.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct VoteResponse {
    /// The current term of the responding node, for the candidate to update itself.
    #[prost(uint64, required, tag="1")]
    pub term: u64,
    /// Will be true if the candidate received a vote from the responder.
    #[prost(bool, required, tag="2")]
    pub vote_granted: bool,
}
/// Invoked by leader to send chunks of a snapshot to a follower (§7).
///
/// Leaders always send chunks in order.
///
/// Receiver implementation:
/// 1. Reply immediately if `term` is less than receiver's current `term`.
/// 2. Create a new snapshot file if snapshot received is the first chunk
///    of the sanpshot (offset is 0).
/// 3. Write data into snapshot file at given offset.
/// 4. Reply and wait for more data chunks if `done` is `false`.
/// 5. Save snapshot file, discard any existing or partial snapshot with a smaller index.
/// 6. If existing log entry has same index and term as snapshot’s last included entry,
///    retain log entries following it and reply.
/// 7. Discard the entire log.
/// 8. Reset state machine using snapshot contents and load snapshot’s cluster configuration.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct InstallSnapshotRequest {
    /// The leader's current term.
    #[prost(uint64, required, tag="1")]
    pub term: u64,
    /// The leader's ID. Useful in redirecting clients.
    #[prost(uint64, required, tag="2")]
    pub leader_id: u64,
    /// The snapshot replaces all log entries up through and including this index.
    #[prost(uint64, required, tag="3")]
    pub last_included_index: u64,
    /// The term of the `last_included_index`.
    #[prost(uint64, required, tag="4")]
    pub last_included_term: u64,
    /// The byte offset where chunk is positioned in the snapshot file.
    #[prost(uint64, required, tag="5")]
    pub offset: u64,
    /// The raw bytes of the snapshot chunk, starting at `offset`.
    #[prost(bytes, required, tag="6")]
    pub data: std::vec::Vec<u8>,
    /// Will be `true` if this is the last chunk in the snapshot.
    #[prost(bool, required, tag="7")]
    pub done: bool,
}
/// An RPC response to an `InstallSnapshotResponse` message.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct InstallSnapshotResponse {
    /// The receiving node's current term, for leader to update itself.
    #[prost(uint64, required, tag="1")]
    pub term: u64,
}
/// The different types of Raft log entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum EntryType {
    /// A normal Raft data entry.
    EntryNormal = 0,
    /// An entry which represents a config change.
    EntryConfigChange = 1,
}
