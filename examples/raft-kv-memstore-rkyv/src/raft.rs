use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use openraft::EntryPayload;
use openraft::entry::RaftEntry;
use openraft::entry::RaftPayload;
use openraft::vote::RaftLeaderId;
use openraft::vote::RaftVote;
use serde::Deserialize;
use serde::Serialize;

use crate::TypeConfig;

/// Represents a node in the Raft cluster, identified by a unique ID and an RPC address.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Default,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    serde::Serialize,
    serde::Deserialize
)]
pub struct Node {
    /// The unique identifier for this node.
    pub node_id: u64,
    /// The RPC address used to communicate with this node.
    pub rpc_addr: String,
}

/// Type alias for the leader ID, parameterized by the application's type configuration.
pub type LeaderId = openraft::impls::leader_id_std::LeaderId<TypeConfig>;
/// Represents the vote state for a node (leader_id + committed flag).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    serde::Serialize,
    serde::Deserialize
)]
pub struct Vote {
    /// The leader id this vote is for.
    pub leader_id: LeaderId,
    /// Whether this vote has been committed by a quorum.
    pub committed: bool,
}

impl Vote {
    /// Create a new uncommitted vote for the given term and node.
    pub fn new(
        term: <TypeConfig as openraft::RaftTypeConfig>::Term,
        node_id: <TypeConfig as openraft::RaftTypeConfig>::NodeId,
    ) -> Self {
        Self {
            leader_id: LeaderId::new(term, node_id),
            committed: false,
        }
    }

    /// Create a new committed vote for the given term and node.
    pub fn new_committed(
        term: <TypeConfig as openraft::RaftTypeConfig>::Term,
        node_id: <TypeConfig as openraft::RaftTypeConfig>::NodeId,
    ) -> Self {
        Self {
            leader_id: LeaderId::new(term, node_id),
            committed: true,
        }
    }
}

impl RaftVote<TypeConfig> for Vote {
    fn from_leader_id(leader_id: LeaderId, committed: bool) -> Self {
        Self { leader_id, committed }
    }

    fn leader_id(&self) -> &LeaderId {
        &self.leader_id
    }

    fn is_committed(&self) -> bool {
        self.committed
    }
}

impl PartialOrd for Vote {
    fn partial_cmp(&self, other: &Vote) -> Option<Ordering> {
        match PartialOrd::partial_cmp(&self.leader_id, &other.leader_id) {
            Some(Ordering::Equal) => PartialOrd::partial_cmp(&self.committed, &other.committed),
            None => match (self.committed, other.committed) {
                (false, false) => None,
                (true, false) => Some(Ordering::Greater),
                (false, true) => Some(Ordering::Less),
                (true, true) => None,
            },
            cmp => cmp,
        }
    }
}

impl fmt::Display for Vote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}:{}>", self.leader_id, if self.committed { "Q" } else { "-" })
    }
}
/// Type alias for a log ID, parameterized by the application's type configuration.
pub type LogId = openraft::LogId<TypeConfig>;
/// Type alias for cluster membership configuration, parameterized by the application's type
/// configuration.
pub type Membership = openraft::Membership<TypeConfig>;

/// A request to set a key-value pair in the state machine.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize
)]
pub struct SetRequest {
    /// The key to set.
    pub key: String,
    /// The value to associate with the key.
    pub value: String,
}

impl fmt::Display for SetRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Set {{ key: {}, value: {} }}", self.key, self.value)
    }
}

/// The response returned after applying a request to the state machine.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize
)]
pub struct Response {
    /// The value associated with the requested key, if any.
    pub value: Option<String>,
}

/// A Raft log entry that can carry application data or a membership change.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize
)]
pub struct Entry {
    /// The log ID (term + index) identifying this entry's position in the log.
    pub log_id: LogId,
    /// The application-level data carried by this entry, if it is a normal entry.
    pub app_data: Option<SetRequest>,
    /// The membership configuration carried by this entry, if it is a membership change entry.
    pub membership: Option<Membership>,
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Determine the payload type for display purposes.
        let payload = if self.app_data.is_some() {
            "normal"
        } else if self.membership.is_some() {
            "membership"
        } else {
            "blank"
        };
        write!(f, "{}({payload})", self.log_id)
    }
}

impl RaftPayload<TypeConfig> for Entry {
    /// Returns the membership configuration from this entry, if present.
    fn get_membership(&self) -> Option<Membership> {
        self.membership.clone()
    }
}

impl RaftEntry<TypeConfig> for Entry {
    /// Constructs a new `Entry` from a log ID and an entry payload.
    fn new(log_id: LogId, payload: EntryPayload<TypeConfig>) -> Self {
        match payload {
            // Blank entries carry no data or membership change.
            EntryPayload::Blank => Self {
                log_id,
                app_data: None,
                membership: None,
            },
            // Normal entries carry application-level data.
            EntryPayload::Normal(app_data) => Self {
                log_id,
                app_data: Some(app_data),
                membership: None,
            },
            // Membership entries carry a new cluster membership configuration.
            EntryPayload::Membership(membership) => Self {
                log_id,
                app_data: None,
                membership: Some(membership),
            },
        }
    }

    /// Returns the parts of the log ID: the committed leader ID and the log index.
    fn log_id_parts(&self) -> (&openraft::vote::leader_id_std::CommittedLeaderId<TypeConfig>, u64) {
        (self.log_id.committed_leader_id(), self.log_id.index())
    }

    /// Updates the log ID of this entry.
    fn set_log_id(&mut self, new: LogId) {
        self.log_id = new;
    }
}

/// A request from a candidate to a peer node asking for a vote in a leader election.
pub struct VoteRequest {
    /// The candidate's current vote (term + candidate ID).
    pub vote: Vote,
    /// The log ID of the last entry in the candidate's log.
    pub last_log_id: LogId,
}

/// The response to a `VoteRequest`.
pub struct VoteResponse {
    /// The responding node's current vote.
    pub vote: Vote,
    /// Whether the vote was granted to the candidate.
    pub vote_granted: bool,
    /// The log ID of the last entry in the responding node's log.
    pub last_log_id: LogId,
}

/// A request from the leader to replicate log entries to a follower.
pub struct AppendEntriesRequest {
    /// The leader's current vote (used to verify leadership authority).
    pub vote: Vote,
    /// The log ID of the entry immediately preceding the new entries.
    pub prev_log_id: LogId,
    /// The log entries to append to the follower's log.
    pub entries: Vec<Entry>,
    /// The leader's current commit index.
    pub leader_commit: LogId,
}

/// The response to an `AppendEntriesRequest`.
pub struct AppendEntriesResponse {
    /// If the request was rejected, contains the vote that caused the rejection.
    pub rejected_by: Option<Vote>,
    /// Whether there was a log conflict detected during the append operation.
    pub conflict: bool,
    /// The log ID of the last entry in the follower's log after processing the request.
    pub last_log_id: Option<LogId>,
}

/// Metadata associated with a snapshot transfer request.
pub struct SnapshotRequestMeta {
    /// The leader's current vote.
    pub vote: Vote,
    /// The log ID of the last entry included in the snapshot.
    pub last_log_id: LogId,
    /// The log ID of the last membership change entry included in the snapshot.
    pub last_membership_log_id: LogId,
    /// The cluster membership configuration at the time the snapshot was taken.
    pub last_membership: Membership,
    /// A unique identifier for this snapshot.
    pub snapshot_id: String,
}

/// The payload of a snapshot request, which is either metadata or a data chunk.
pub enum SnapshotRequestPayload {
    /// Snapshot metadata sent at the beginning of a snapshot transfer.
    Meta(SnapshotRequestMeta),
    /// A chunk of raw snapshot data.
    Chunk(Vec<u8>),
}

/// A request to install a snapshot on a follower node.
pub struct SnapshotRequest {
    /// The payload of the snapshot request (either metadata or a data chunk).
    pub payload: SnapshotRequestPayload,
}

/// The response to a `SnapshotRequest`.
pub struct SnapshotResponse {
    /// The responding node's current vote.
    pub vote: Vote,
}

/// The in-memory state of the application's state machine.
pub struct StateMachineData {
    /// The log ID of the last entry that has been applied to the state machine.
    pub last_applied: LogId,
    /// The key-value store representing the current application state.
    pub data: HashMap<String, String>,
    /// The log ID of the last membership change that has been applied.
    pub last_membership_log_id: LogId,
    /// The most recently applied cluster membership configuration.
    pub last_membership: Membership,
}
