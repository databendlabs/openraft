//! The RaftStorage interface and message types.

use std::sync::Arc;

use actix::{
    dev::ToEnvelope,
    prelude::*,
};
use futures::sync::{mpsc::UnboundedReceiver, oneshot::Sender};
use serde::{Serialize, Deserialize};

use crate::{
    NodeId, AppError,
    messages,
};

//////////////////////////////////////////////////////////////////////////////
// GetInitialState ///////////////////////////////////////////////////////////

/// A request from Raft to get Raft's state information from storage.
///
/// When the Raft actor is first started, it will call this interface on the storage system to
/// fetch the last known state from stable storage. If no such entry exists due to being the
/// first time the node has come online, then the default value for `InitialState` should be used.
///
/// ### pro tip
/// The storage impl may need to look in a few different places to accurately respond to this
/// request. That last entry in the log for `last_log_index` & `last_log_term`; the node's hard
/// state record; and the index of the last log applied to the state machine.
pub struct GetInitialState<E: AppError> {
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> GetInitialState<E> {
    // Create a new instance.
    pub fn new() -> Self {
        Self{marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for GetInitialState<E> {
    type Result = Result<InitialState, E>;
}

/// A struct used to represent the initial state which a Raft node needs when first starting.
#[derive(Clone, Debug)]
pub struct InitialState {
    /// The index of the last entry.
    pub last_log_index: u64,
    /// The term of the last log entry.
    pub last_log_term: u64,
    /// The index of the last log applied to the state machine.
    pub last_applied_log: u64,
    /// The saved hard state of the node.
    pub hard_state: HardState,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// GetLogEntries /////////////////////////////////////////////////////////////////////////////////

/// A request from Raft to get a series of log entries from storage.
///
/// The start value is inclusive in the search and the stop value is non-inclusive:
/// `[start, stop)`.
pub struct GetLogEntries<E: AppError> {
    pub start: u64,
    pub stop: u64,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> GetLogEntries<E> {
    // Create a new instance.
    pub fn new(start: u64, stop: u64) -> Self {
        Self{start, stop, marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for GetLogEntries<E> {
    type Result = Result<Vec<messages::Entry>, E>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// AppendLogEntry ////////////////////////////////////////////////////////////////////////////////

/// A request from Raft to append a new entry to the log.
///
/// These requests come about via client requests, and as such, this is the only RaftStorage
/// interface which is allowed to return errors which will not cause Raft to shutdown. Application
/// errors coming from this interface will be sent back as-is to the call point where your
/// application originally presented the client request to Raft.
///
/// This property of error handling allows you to keep your application logic as close to the
/// storage layer as needed.
pub struct AppendLogEntry<E: AppError> {
    pub entry: Arc<messages::Entry>,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> AppendLogEntry<E> {
    // Create a new instance.
    pub fn new(entry: Arc<messages::Entry>) -> Self {
        Self{entry, marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for AppendLogEntry<E> {
    type Result = Result<(), E>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ReplicateLogEntries ///////////////////////////////////////////////////////////////////////////

/// A request from Raft to replicate the payload of entries to the log.
///
/// These requests come about via the Raft leader's replication process. An error coming from this
/// interface will cause Raft to shutdown, as this is not where application logic should be
/// returning application specific errors. Application specific constraints may only be enforced
/// in the `AppendLogEntry` handler.
///
/// Though the entries will always be presented in order, each entry's index should be used to
/// determine its location to be written in the log, as logs may need to be overwritten under
/// some circumstances.
pub struct ReplicateLogEntries<E: AppError> {
    pub entries: Arc<Vec<messages::Entry>>,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> ReplicateLogEntries<E> {
    // Create a new instance.
    pub fn new(entries: Arc<Vec<messages::Entry>>) -> Self {
        Self{entries, marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for ReplicateLogEntries<E> {
    type Result = Result<(), E>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ApplyToStateMachine ///////////////////////////////////////////////////////////////////////////

/// A request from Raft to apply the given log entries to the state machine.
///
/// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
/// have been replicated to a majority of the cluster, will be applied to the state machine.
///
/// NOTE WELL: once the futures ecosystem settles a bit and we can pass around references in
/// futures and message types, this interface will solidify and payload will always just be a
/// `&[Entry]` or the like. For now, the payload variants help to keep allocations lower.
pub struct ApplyToStateMachine<E: AppError> {
    pub payload: ApplyToStateMachinePayload,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> ApplyToStateMachine<E> {
    // Create a new instance.
    pub fn new(payload: ApplyToStateMachinePayload) -> Self {
        Self{payload, marker: std::marker::PhantomData}
    }
}

/// The type of payload which needs to be applied to the state machine.
pub enum ApplyToStateMachinePayload {
    Multi(Vec<messages::Entry>),
    Single(Arc<messages::Entry>),
}

impl<E: AppError> Message for ApplyToStateMachine<E> {
    type Result = Result<(), E>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// CreateSnapshot ////////////////////////////////////////////////////////////////////////////////

/// A request from Raft to have a new snapshot created which covers the current breadth
/// of the log.
///
/// The Raft node guarantees that this interface will never be called multiple overlapping times
/// from the same Raft node, and it will not be called when an `InstallSnapshot` operation is in
/// progress.
///
/// **It is critical to note** that the newly created snapshot must be able to be used to
/// completely and accurately create a state machine. In addition to saving space on disk (log
/// compaction), snapshots are used to bring new Raft nodes and slow Raft nodes up-to-speed with
/// the cluster leader.
///
/// ### implementation algorithm
/// - The generated snapshot should include all log entries starting from entry `0` up through
/// the index specified by `through`. This will include any snapshot which may already exist. If
/// a snapshot does already exist, the new log compaction process should be able to just load the
/// old snapshot first, and resume processing from its last entry.
/// - The newly generated snapshot should be written to the configured snapshot directory.
/// - All previous entries in the log should be deleted up to the entry specified at index
/// `through`.
/// - The entry at index `through` should be replaced with a new entry created from calling
/// `actix_raft::messages::Entry::new_snapshot_pointer(...)`.
/// - Any old snapshot will no longer have representation in the log, and should be deleted.
/// - Return a `CurrentSnapshotData` struct which contains all metadata pertinent to the snapshot.
pub struct CreateSnapshot<E: AppError> {
    /// The new snapshot should start from entry `0` and should cover all entries through the
    /// index specified here, inclusive.
    pub through: u64,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> CreateSnapshot<E> {
    // Create a new instance.
    pub fn new(through: u64) -> Self {
        Self{through, marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for CreateSnapshot<E> {
    type Result = Result<CurrentSnapshotData, E>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// InstallSnapshot ///////////////////////////////////////////////////////////////////////////////

/// A request from Raft to have a new snapshot written to disk and installed.
///
/// This message holds an `UnboundedReceiver` which will stream in new chunks of data as they are
/// received from the Raft leader.
///
/// ### implementation algorithm
/// - Upon receiving the request, a new snapshot file should be created on disk.
/// - Every new chunk of data received should be written to the new snapshot file starting at the
/// `offset` specified in the chunk.
/// - If the receiver is dropped, the snapshot which was being created should be removed from
/// disk.
///
/// Once a chunk is received which is the final chunk of the snapshot, after writing the data,
/// there are a few important steps to take:
///
/// - Create a new entry in the log via the `actix_raft::messages::Entry::new_snapshot_pointer(...)`
/// constructor. Insert the new entry into the log at the specified `index` of this payload.
/// - If there are any logs older than `index`, remove them.
/// - If there are any other snapshots in the configured snapshot dir, remove them.
/// - If existing log entry has same index and term as snapshot's last included entry, retain log
/// entries following it, then return.
/// - Else, discard the entire log leaving only the new snapshot pointer. The state machine must
/// be rebuilt from the new snapshot. Return once the state machine has been brought up-to-date.
pub struct InstallSnapshot<E: AppError> {
    /// The term which the final entry of this snapshot covers.
    pub term: u64,
    /// The index of the final entry which this snapshot covers.
    pub index: u64,
    /// A stream of data chunks for this snapshot.
    pub stream: UnboundedReceiver<InstallSnapshotChunk>,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> InstallSnapshot<E> {
    // Create a new instance.
    pub fn new(term: u64, index: u64, stream: UnboundedReceiver<InstallSnapshotChunk>) -> Self {
        Self{term, index, stream, marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for InstallSnapshot<E> {
    type Result = Result<(), E>;
}

/// A chunk of snapshot data.
pub struct InstallSnapshotChunk {
    /// The byte offset where chunk is positioned in the snapshot file.
    pub offset: u64,
    /// The raw bytes of the snapshot chunk, starting at `offset`.
    pub data: Vec<u8>,
    /// Will be `true` if this is the last chunk in the snapshot.
    pub done: bool,
    /// A callback channel to indicate when the chunk has been successfully written.
    pub cb: Sender<()>,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// GetCurrentSnapshot ////////////////////////////////////////////////////////////////////////////

/// A request from Raft to get metadata of the current snapshot.
///
/// ### implementation algorithm
/// Implementation for this type's handler should be quite simple. Check the configured snapshot
/// directory for any snapshot files. A proper implementation will only ever have one
/// active snapshot, though another may exist while it is being created. As such, it is
/// recommended to use a file naming pattern which will allow for easily distinguishing betweeen
/// the current live snapshot, and any new snapshot which is being created.
pub struct GetCurrentSnapshot<E: AppError> {
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> GetCurrentSnapshot<E> {
    // Create a new instance.
    pub fn new() -> Self {
        Self{marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for GetCurrentSnapshot<E> {
    type Result = Result<Option<CurrentSnapshotData>, E>;
}

/// The data associated with the current snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentSnapshotData {
    /// The snapshot entry's term.
    pub term: u64,
    /// The snapshot entry's index.
    pub index: u64,
    /// The latest membership configuration covered by the snapshot.
    pub membership: messages::MembershipConfig,
    /// The snapshot entry's pointer to the snapshot file.
    pub pointer: messages::EntrySnapshotPointer,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// SaveHardState /////////////////////////////////////////////////////////////////////////////////

/// A request from Raft to save its HardState.
pub struct SaveHardState<E: AppError>{
    pub hs: HardState,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> SaveHardState<E> {
    // Create a new instance.
    pub fn new(hs: HardState) -> Self {
        Self{hs, marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for SaveHardState<E> {
    type Result = Result<(), E>;
}

/// A record holding the hard state of a Raft node.
///
/// This model derives serde's traits for easily (de)serializing this
/// model for storage & retrieval.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HardState {
    /// The last recorded term observed by this system.
    pub current_term: u64,
    /// The ID of the node voted for in the `current_term`.
    pub voted_for: Option<NodeId>,
    /// The cluster membership configuration.
    pub membership: messages::MembershipConfig,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RaftStorage ///////////////////////////////////////////////////////////////////////////////////

/// A trait defining the interface of a Raft storage actor.
///
/// ### implementation notes
/// Appending log entries should not be considered complete until the data has been flushed to
/// disk. Some of Raft's safety guarantees are premised upon committed log entries being fully
/// flushed to disk. If this invariant is not upheld, the system could incur data loss.
///
/// ### snapshot
/// See §7.
///
/// Each node in the cluster will independently snapshot its data for compaction purposes. The
/// conditions for when a new snapshot will be generated is based on the nodes `Config`. In
/// addition to periodic snapshots, a leader may need to send an `InstallSnapshot` RPC to
/// followers which are far behind or which are new to the cluster. This is based on the same
/// `Config` value. The Raft node will send a message to this `RaftStorage` interface when a
/// periodic snapshot is to be generated based on its configuration.
///
/// Log compaction, which is part of what taking a snapshot is for, is an application specific
/// process. The essential idea is that superfluous records in the log will be removed. See §7 for
/// more details. There are a few snapshot related messages which the `RaftStorage` actor must
/// handle:
///
/// - `CreateSnapshot`: a request to create a new snapshot of the current log.
/// - `InstallSnapshot`: the Raft leader is streaming over a snapshot, install it.
/// - `GetCurrentSnapshot`: the Raft node needs to know the location of the current snapshot.
///
/// See each message type for more details on the message and how to properly implement their
/// behaviors.
pub trait RaftStorage<E>
    where
        E: AppError,
        Self: Actor<Context=Context<Self>>,

        Self: Handler<GetInitialState<E>>,
        Self::Context: ToEnvelope<Self, GetInitialState<E>>,

        Self: Handler<SaveHardState<E>>,
        Self::Context: ToEnvelope<Self, SaveHardState<E>>,

        Self: Handler<GetLogEntries<E>>,
        Self::Context: ToEnvelope<Self, GetLogEntries<E>>,

        Self: Handler<AppendLogEntry<E>>,
        Self::Context: ToEnvelope<Self, AppendLogEntry<E>>,

        Self: Handler<ReplicateLogEntries<E>>,
        Self::Context: ToEnvelope<Self, ReplicateLogEntries<E>>,

        Self: Handler<ApplyToStateMachine<E>>,
        Self::Context: ToEnvelope<Self, ApplyToStateMachine<E>>,

        Self: Handler<CreateSnapshot<E>>,
        Self::Context: ToEnvelope<Self, CreateSnapshot<E>>,

        Self: Handler<InstallSnapshot<E>>,
        Self::Context: ToEnvelope<Self, InstallSnapshot<E>>,

        Self: Handler<GetCurrentSnapshot<E>>,
        Self::Context: ToEnvelope<Self, GetCurrentSnapshot<E>>,
{
    /// Create a new instance which will store its snapshots in the given directory.
    ///
    /// The values given to this constructor should only be used when the node is coming online
    /// for the first time. Otherwise the persistent storage should always take precedence.
    ///
    /// The value given for `members` is used as the initial cluster config when the node comes
    /// online for the first time.
    fn new(members: Vec<NodeId>, snapshot_dir: String) -> Self;
}
