//! All public facing message types.
//!
//! For users of this Raft implementation, this module defines the data types of this crate's API.
//! The `RaftNetwork` trait is based entirely off of these messages, and communication with the
//! Raft actor is based entirely off of these messages other than the admin messages.

use actix::prelude::*;
use serde::{Serialize, Deserialize};

use crate::{AppError, NodeId};

//////////////////////////////////////////////////////////////////////////////////////////////////
// AppendEntriesRequest //////////////////////////////////////////////////////////////////////////

/// An RPC invoked by the leader to replicate log entries (§5.3); also used as heartbeat (§5.2).
///
/// ### actix::Message
/// Applications using this Raft implementation are responsible for implementing the
/// networking/transport layer which must move RPCs between nodes. Once the application instance
/// recieves a Raft RPC, it must send the RPC to the Raft node via its `actix::Addr` and then
/// return the response to the original sender.
///
/// The result type of calling the Raft actor with this message type is
/// `Result<AppendEntriesResponse, ()>`. The Raft spec assigns no significance to failures during
/// the handling or sending of RPCs and all RPCs are handled in an idempotent fashion, so Raft
/// will almost always retry sending a failed RPC, depending on the state of the Raft.
#[derive(Debug, Serialize, Deserialize)]
pub struct AppendEntriesRequest {
    /// A non-standard field, this is the ID of the intended recipient of this RPC.
    pub target: u64,
    /// The leader's current term.
    pub term: u64,
    /// The leader's ID. Useful in redirecting clients.
    pub leader_id: u64,
    /// The index of the log entry immediately preceding the new entries.
    pub prev_log_index: u64,
    /// The term of the `prev_log_index` entry.
    pub prev_log_term: u64,
    /// The new log entries to store.
    ///
    /// This may be empty when the leader is sending heartbeats. Entries
    /// may be batched for efficiency.
    pub entries: Vec<Entry>,
    /// The leader's commit index.
    pub leader_commit: u64,
}

impl Message for AppendEntriesRequest {
    /// The result type of this message.
    ///
    /// The `Result::Err` type is `()` as Raft assigns no significance to RPC failures, they will
    /// be retried almost always as long as permitted by the current state of the Raft.
    type Result = Result<AppendEntriesResponse, ()>;
}

/// An RPC response to an `AppendEntriesRequest` message.
#[derive(Debug, Serialize, Deserialize)]
pub struct AppendEntriesResponse {
    /// The responding node's current term, for leader to update itself.
    pub term: u64,
    /// Will be true if follower contained entry matching `prev_log_index` and `prev_log_term`.
    pub success: bool,
    /// A value used to implement the _conflicting term_ optimization outlined in §5.3.
    ///
    /// This value will only be present, and should only be considered, when `success` is `false`.
    pub conflict_opt: Option<ConflictOpt>,
}

/// A struct used to implement the _conflicting term_ optimization outlined in §5.3 for log replication.
///
/// This value will only be present, and should only be considered, when an `AppendEntriesResponse`
/// object has a `success` value of `false`.
///
/// This implementation of Raft uses this value to more quickly synchronize a leader with its
/// followers which may be some distance behind in replication, may have conflicting entries, or
/// which may be new to the cluster.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConflictOpt {
    /// The term of the most recent entry which does not conflict with the received request.
    pub term: u64,
    /// The index of the most recent entry which does not conflict with the received request.
    pub index: u64,
}

/// A Raft log entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// This entry's term.
    pub term: u64,
    /// This entry's index.
    pub index: u64,
    /// This entry's type.
    pub entry_type: EntryType,
}

/// Log entry type variants.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EntryType {
    /// A normal log entry.
    Normal(EntryNormal),
    /// A config change log entry.
    ConfigChange(EntryConfigChange),
    /// An entry which points to a snapshot.
    SnapshotPointer(EntrySnapshotPointer),
}

/// A normal log entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntryNormal {
    /// The contents of this entry.
    pub data: Vec<u8>,
}

/// A config change log entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntryConfigChange {
    /// The full list of node IDs to be considered cluster members as part of this config change.
    pub members: Vec<u64>,
    /// Any application specific supplemental data asscoiated with this config change.
    pub supplemental: Option<Vec<u8>>,
}

/// An entry which points to a snapshot.
///
/// This will only be present when read from storage. An entry of this type will never be
/// transmitted from a leader during replication, an `InstallSnapshotRequest`
/// RPC will be sent instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntrySnapshotPointer {
    /// The location of the snapshot file on disk.
    pub path: String,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// VoteRequest ///////////////////////////////////////////////////////////////////////////////////

/// An RPC invoked by candidates to gather votes (§5.2).
///
/// ### actix::Message
/// Applications using this Raft implementation are responsible for implementing the
/// networking/transport layer which must move RPCs between nodes. Once the application instance
/// recieves a Raft RPC, it must send the RPC to the Raft node via its `actix::Addr` and then
/// return the response to the original sender.
///
/// The result type of calling the Raft actor with this message type is `Result<VoteResponse, ()>`.
/// The Raft spec assigns no significance to failures during the handling or sending of RPCs and
/// all RPCs are handled in an idempotent fashion, so Raft will almost always retry sending a
/// failed RPC, depending on the state of the Raft.
#[derive(Debug, Serialize, Deserialize)]
pub struct VoteRequest {
    /// A non-standard field, this is the ID of the intended recipient of this RPC.
    pub target: u64,
    /// The candidate's current term.
    pub term: u64,
    /// The candidate's ID.
    pub candidate_id: u64,
    /// The index of the candidate’s last log entry (§5.4).
    pub last_log_index: u64,
    /// The term of the candidate’s last log entry (§5.4).
    pub last_log_term: u64,
}

impl Message for VoteRequest {
    /// The result type of this message.
    ///
    /// The `Result::Err` type is `()` as Raft assigns no significance to RPC failures, they will
    /// be retried almost always as long as permitted by the current state of the Raft.
    type Result = Result<VoteResponse, ()>;
}

impl VoteRequest {
    /// Create a new instance.
    pub fn new(target: u64, term: u64, candidate_id: u64, last_log_index: u64, last_log_term: u64) -> Self {
        Self{target, term, candidate_id, last_log_index, last_log_term}
    }
}

/// An RPC response to an `VoteResponse` message.
#[derive(Debug, Serialize, Deserialize)]
pub struct VoteResponse {
    /// The current term of the responding node, for the candidate to update itself.
    pub term: u64,
    /// Will be true if the candidate received a vote from the responder.
    pub vote_granted: bool,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// InstallSnapshotRequest ////////////////////////////////////////////////////////////////////////

/// Invoked by leader to send chunks of a snapshot to a follower (§7).
///
/// ### actix::Message
/// Applications using this Raft implementation are responsible for implementing the
/// networking/transport layer which must move RPCs between nodes. Once the application instance
/// recieves a Raft RPC, it must send the RPC to the Raft node via its `actix::Addr` and then
/// return the response to the original sender.
///
/// The result type of calling the Raft actor with this message type is
/// `Result<InstallSnapshotResponse, ()>`. The Raft spec assigns no significance to failures during
/// the handling or sending of RPCs and all RPCs are handled in an idempotent fashion, so Raft will
/// almost always retry sending a failed RPC, depending on the state of the Raft.
#[derive(Debug, Serialize, Deserialize)]
pub struct InstallSnapshotRequest {
    /// A non-standard field, this is the ID of the intended recipient of this RPC.
    pub target: u64,
    /// The leader's current term.
    pub term: u64,
    /// The leader's ID. Useful in redirecting clients.
    pub leader_id: u64,
    /// The snapshot replaces all log entries up through and including this index.
    pub last_included_index: u64,
    /// The term of the `last_included_index`.
    pub last_included_term: u64,
    /// The byte offset where chunk is positioned in the snapshot file.
    pub offset: u64,
    /// The raw Vec<u8> of the snapshot chunk, starting at `offset`.
    pub data: Vec<u8>,
    /// Will be `true` if this is the last chunk in the snapshot.
    pub done: bool,
}

impl Message for InstallSnapshotRequest {
    /// The result type of this message.
    ///
    /// The `Result::Err` type is `()` as Raft assigns no significance to RPC failures, they will
    /// be retried almost always as long as permitted by the current state of the Raft.
    type Result = Result<InstallSnapshotResponse, ()>;
}

/// An RPC response to an `InstallSnapshotResponse` message.
#[derive(Debug, Serialize, Deserialize)]
pub struct InstallSnapshotResponse {
    /// The receiving node's current term, for leader to update itself.
    pub term: u64,
}
//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientPayload /////////////////////////////////////////////////////////////////////////////////

/// A payload of entries coming from a client request.
///
/// The entries of this payload will be appended to the Raft log and then applied to the Raft state
/// machine according to the Raft protocol.
///
/// ### actix::Message
/// Applications using this Raft implementation are responsible for implementing the
/// networking/transport layer which must move RPCs between nodes. Once the application instance
/// recieves a Raft RPC, it must send the RPC to the Raft node via its `actix::Addr` and then
/// return the response to the original sender.
///
/// The result type of calling the Raft actor with this message type is
/// `Result<ClientPayloadResponse, StorageError>`. Applications built around this implementation of
/// Raft will often need to perform their own custom logic in the storage layer and often times it
/// is critical to be able to surface such errors to the application and its clients. To meet that
/// end, `ClientError` allows for the communication of application specific errors. This is
/// defined in protobuf to allow for more easily forwarding client requests to the Raft master when
/// needed.
#[derive(Debug, Serialize, Deserialize)]
pub struct ClientPayload<E: AppError> {
    /// The application specific contents of this client request.
    pub entry: EntryNormal,
    /// The response mode needed by this request.
    pub response_mode: ResponseMode,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> ClientPayload<E> {
    /// Create a new instance.
    pub fn new(entry: EntryNormal, response_mode: ResponseMode) -> Self {
        Self{entry, response_mode, marker: std::marker::PhantomData}
    }

    /// Generate a new blank payload.
    ///
    /// This is primarily used by new leaders when first coming to power.
    pub(crate) fn new_blank_payload() -> Self {
        Self::new(EntryNormal{data: vec![]}, ResponseMode::Applied)
    }
}

impl<E: AppError> Message for ClientPayload<E> {
    /// The result type of this message.
    type Result = Result<ClientPayloadResponse, ClientError<E>>;
}

/// The desired response mode for a client request.
///
/// This value specifies when a client request desires to receive its response from Raft. When
/// `Comitted` is chosen, the client request will receive a response after the request has been
/// successfully replicated to at least half of the nodes in the cluster. This is what the Raft
/// protocol refers to as being comitted.
///
/// When `Applied` is chosen, the client request will receive a response after the request has
/// been successfully committed and successfully applied to the state machine.
///
/// The choice between these two options depends on the requirements related to the request. If
/// the data of the client request payload will need to be read immediately after the response is
/// received, then `Applied` must be used. If there is no requirement that the data must be
/// immediately read after receiving a response, then `Committed` may be used to speed up response
/// times for data mutating requests.
#[derive(Debug, Serialize, Deserialize)]
pub enum ResponseMode {
    /// A response will be returned after the request has been committed to the cluster.
    Committed,
    /// A response will be returned  after the request has been applied to the leader's state machine.
    Applied,
}

/// A response to a client payload proposed to the Raft system.
#[derive(Debug, Serialize, Deserialize)]
pub struct ClientPayloadResponse {
    /// The log index of the successfully process client request.
    pub index: u64,
}

/// Error variants which may arise while handling client requests.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag="type")]
pub enum ClientError<E: AppError> {
    /// Some error which has taken place internally in Raft.
    Internal,
    /// An application specific error.
    #[serde(bound="E: AppError")]
    Application(E),
    /// The Raft node returning this error is not the Raft leader.
    ///
    /// Forward the payload to the specified leader. If the leader is unknown, it is up to the
    /// application to determine how to handle. The payload can be buffered in the app until the
    /// new leader is known, or it can be returned to the client as an error and the client can be
    /// instructed to send to a new random node until the leader is known.
    ///
    /// The process of electing a new leader is usually a very fast process in Raft, so buffering
    /// the client payload until the new leader is known should not cause a lot of overhead.
    #[serde(bound="E: AppError")]
    ForwardToLeader {
        /// The original payload which this error is associated with.
        payload: ClientPayload<E>,
        /// The ID of the current Raft leader, if known.
        leader: Option<NodeId>,
    },
}

impl<E: AppError> std::fmt::Display for ClientError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Internal => write!(f, "An internal error was encountered in Raft."),
            ClientError::Application(err) => write!(f, "{}", &err),
            ClientError::ForwardToLeader{..} => write!(f, "The client payload must be forwarded to the Raft leader for processing."),
        }
    }
}

impl<E: AppError> std::error::Error for ClientError<E> {}

