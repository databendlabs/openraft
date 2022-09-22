use serde::Deserialize;
use serde::Serialize;

use super::AppData;
use super::AppDataResponse;
use super::Entry;
use super::LogId;
use super::Membership;
use super::SnapshotMeta;

/// An RPC sent by candidates to gather votes (§5.2).
#[derive(Debug, Serialize, Deserialize)]
pub struct VoteRequest {
    /// The candidate's current term.
    pub term: u64,

    pub candidate_id: u64,

    pub last_log_id: Option<LogId>,
}

/// The response to a `VoteRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct VoteResponse {
    /// The current term of the responding node, for the candidate to update itself.
    pub term: u64,

    /// Will be true if the candidate received a vote from the responder.
    pub vote_granted: bool,

    /// The last log id stored on the remote voter.
    pub last_log_id: Option<LogId>,
}

/// An RPC sent by a cluster leader to replicate log entries (§5.3), and as a heartbeat (§5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEntriesRequest<D: AppData> {
    /// The leader's current term.
    pub term: u64,

    /// The leader's ID. Useful in redirecting clients.
    pub leader_id: u64,

    pub prev_log_id: Option<LogId>,

    /// The new log entries to store.
    ///
    /// This may be empty when the leader is sending heartbeats. Entries
    /// are batched for efficiency.
    #[serde(bound = "D: AppData")]
    pub entries: Vec<Entry<D>>,

    /// The leader's committed log id.
    pub leader_commit: Option<LogId>,
}

/// The response to an `AppendEntriesRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct AppendEntriesResponse {
    /// The responding node's current term, for leader to update itself.
    pub term: u64,

    pub success: bool,
    pub conflict: bool,
}

/// An RPC sent by the Raft leader to send chunks of a snapshot to a follower (§7).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallSnapshotRequest {
    /// The leader's current term.
    pub term: u64,
    /// The leader's ID. Useful in redirecting clients.
    pub leader_id: u64,

    /// Metadata of a snapshot: snapshot_id, last_log_ed membership etc.
    pub meta: SnapshotMeta,

    /// The byte offset where this chunk of data is positioned in the snapshot file.
    pub offset: u64,
    /// The raw bytes of the snapshot chunk, starting at `offset`.
    pub data: Vec<u8>,

    /// Will be `true` if this is the last chunk in the snapshot.
    pub done: bool,
}

/// The response to an `InstallSnapshotRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct InstallSnapshotResponse {
    /// The receiving node's current term, for leader to update itself.
    pub term: u64,

    /// The last applied log id after snapshot being installed.
    ///
    /// A node may choose not to install a snapshot if it already has a greater `last_applied`.
    /// In this case, it just returns the `last_applied`.
    pub last_applied: Option<LogId>,
}

/// The response to a `ClientRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ClientWriteResponse<R: AppDataResponse> {
    pub log_id: LogId,

    /// Application specific response data.
    #[serde(bound = "R: AppDataResponse")]
    pub data: R,

    /// If the log entry is a change-membership entry.
    pub membership: Option<Membership>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddLearnerResponse {
    pub matched: Option<LogId>,
}
