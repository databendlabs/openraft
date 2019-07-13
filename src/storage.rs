//! The RaftStorage interface and message types.

use std::sync::Arc;

use actix::{
    dev::ToEnvelope,
    prelude::*,
};
use futures::sync::mpsc::UnboundedReceiver;

use crate::{
    NodeId, AppError,
    messages,
};

//////////////////////////////////////////////////////////////////////////////
// GetInitialState ///////////////////////////////////////////////////////////

/// An actix message type for requesting Raft state information from the storage layer.
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

/// An actix message type for requesting a series of log entries from storage.
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
// AppendLogEntries //////////////////////////////////////////////////////////////////////////////

/// An actix message type for requesting a series of entries to be written to the log.
///
/// Though the entries will always be presented in order, each entry's index should be used for
/// determining its location to be written in the log, as logs may need to be overwritten under
/// some circumstances.
///
/// It is critical that `RaftStorage` implementations honor the value of `mode`. The
/// `AppendLogEntries` interface is the only interface which is allowed to return errors without
/// causing Raft to immediately shutdown to preserve data integrity. However, this is only allowed
/// for mode `Leader`, but is never allowed for mode `Follower`.
///
/// It is also important to note that implementations must ensure that all entries in the payload
/// are successfully applied, or if in mode `Leader`, and an error needs to be returned, that none
/// of the entries are applied. This will only practically take place if the application using
/// Raft supports batch client payloads, as that is the only case where a client request will
/// be presented with multiple entries in a single `AppendLogEntries` call to `RaftStorage`.
pub struct AppendLogEntries<E: AppError> {
    pub mode: AppendLogEntriesMode,
    pub entries: Arc<Vec<messages::Entry>>,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> AppendLogEntries<E> {
    // Create a new instance.
    pub fn new(mode: AppendLogEntriesMode, entries: Arc<Vec<messages::Entry>>) -> Self {
        Self{mode, entries, marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for AppendLogEntries<E> {
    type Result = Result<(), E>;
}

/// A mode indicating if the associated storage command is from replication or a client write to the leader.
///
/// When in mode `Leader`, the associated `AppendLogEntries` is coming about due to a client
/// command on the leader. If application specific business logic needs to be performed,
/// potentially returning an error, it is permitted to do so in this mode. If errors are returned
/// when in this mode, they will be propagated back to the caller as is.
///
/// When in mode `Follower`, the associated `AppendLogEntries` is coming about by way
/// of replication commands from the cluster leader. This must never fail. If an error is returned
/// when in this mode, the Raft node will shut down in order to preserve data inntegrity.
pub enum AppendLogEntriesMode {
    Leader,
    Follower,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ApplyEntriesToStateMachine ////////////////////////////////////////////////////////////////////

/// A request from the Raft node to apply the given log entries to the state machine.
///
/// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
/// have been replicated to a majority of the cluster, will be applied to the state machine.
pub struct ApplyEntriesToStateMachine<E: AppError> {
    pub entries: Arc<Vec<messages::Entry>>,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> ApplyEntriesToStateMachine<E> {
    // Create a new instance.
    pub fn new(entries: Arc<Vec<messages::Entry>>) -> Self {
        Self{entries, marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for ApplyEntriesToStateMachine<E> {
    type Result = Result<(), E>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// CreateSnapshot ////////////////////////////////////////////////////////////////////////////////

/// A request from the Raft node to have a new snapshot created which covers the current breadth
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
/// - The newly generated snapshot should be written to the directory specified by `snapshot_dir`.
/// - All previous entries in the log should be deleted up to the entry specified at index
/// `through`.
/// - The entry at index `through` should be replaced with a new entry created from calling
/// `actix_raft::messages::Entry::new_snapshot_pointer(...)`.
/// - Any old snapshot will no longer have representation in the log, and should be deleted.
/// - Return a copy of the snapshot pointer entry created earlier.
pub struct CreateSnapshot<E: AppError> {
    /// The new snapshot should start from entry `0` and should cover all entries through the
    /// index specified here, inclusive.
    pub through: u64,
    /// The directory where the new snapshot is to be written.
    pub snapshot_dir: String,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> CreateSnapshot<E> {
    // Create a new instance.
    pub fn new(through: u64, snapshot_dir: String) -> Self {
        Self{through, snapshot_dir, marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for CreateSnapshot<E> {
    type Result = Result<messages::Entry, E>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// InstallSnapshot ///////////////////////////////////////////////////////////////////////////////

/// A request from the Raft node to have a new snapshot written to disk and installed.
///
/// This message holds an `UnboundedReceiver` which will stream in new chunks of data as they are
/// received from the Raft leader.
///
/// ### implementation algorithm
/// - Upon receiving the request, a new snapshot file should be created on disk.
/// - Every new chunk of data received should be written to the new snapshot file starting at the
/// `offset` specified in the chunk. The Raft actor will ensure that redelivered chunks are not
/// sent through multiple times.
/// - If the receiver is dropped, the snapshot which was being created should be removed from
/// disk.
///
/// Once a chunk is received which is the final chunk of the snapshot, after writing the data,
/// there are a few important steps to take:
///
/// - Create a new entry in the log via the `actix_raft::messages::Entry::new_snapshot_pointer(...)`
/// constructor. Insert the new entry into the log at the specified `index` of this payload.
/// - If there are any logs older than `index`, remove them.
/// - If there are any other snapshots in `snapshot_dir`, remove them.
/// - If there are any logs newer than `index`, then return.
/// - If there are no logs newer than `index`, then the state machine should be reset, and
/// recreated from the new snapshot. Return once the state machine has been brought up-to-date.
pub struct InstallSnapshot<E: AppError> {
    /// The term which the final entry of this snapshot covers.
    pub term: u64,
    /// The index of the final entry which this snapshot covers.
    pub index: u64,
    /// The directory where the new snapshot is to be written.
    pub snapshot_dir: String,
    /// A stream of data chunks for this snapshot.
    pub stream: UnboundedReceiver<InstallSnapshotChunk>,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> InstallSnapshot<E> {
    // Create a new instance.
    pub fn new(term: u64, index: u64, snapshot_dir: String, stream: UnboundedReceiver<InstallSnapshotChunk>) -> Self {
        Self{term, index, snapshot_dir, stream, marker: std::marker::PhantomData}
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
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// GetCurrentSnapshot ////////////////////////////////////////////////////////////////////////////

/// A request from the Raft node to get metadata of the current snapshot.
///
/// ### implementation algorithm
/// Implementation for this type's handler should be quite simple. Check the directory specified
/// by `snapshot_dir` for any snapshot files. A proper implementation will only ever have one
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
    type Result = Result<Option<GetCurrentSnapshotData>, E>;
}

/// The data associated with the current snapshot.
pub struct GetCurrentSnapshotData {
    /// The snapshot entry's term.
    pub term: u64,
    /// The snapshot entry's index.
    pub index: u64,
    /// The snapshot entry's pointer to the snapshot file.
    pub pointer: messages::EntrySnapshotPointer,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// SaveHardState /////////////////////////////////////////////////////////////////////////////////

/// A request from the Raft node to save its HardState.
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
pub struct HardState {
    /// The last recorded term observed by this system.
    pub current_term: u64,
    /// The ID of the node voted for in the `current_term`.
    pub voted_for: Option<NodeId>,
    /// The IDs of all known members of the cluster.
    pub members: Vec<u64>,
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

        Self: Handler<AppendLogEntries<E>>,
        Self::Context: ToEnvelope<Self, AppendLogEntries<E>>,

        Self: Handler<ApplyEntriesToStateMachine<E>>,
        Self::Context: ToEnvelope<Self, ApplyEntriesToStateMachine<E>>,

        Self: Handler<CreateSnapshot<E>>,
        Self::Context: ToEnvelope<Self, CreateSnapshot<E>>,

        Self: Handler<InstallSnapshot<E>>,
        Self::Context: ToEnvelope<Self, InstallSnapshot<E>>,

        Self: Handler<GetCurrentSnapshot<E>>,
        Self::Context: ToEnvelope<Self, GetCurrentSnapshot<E>>,
{
    /// Create a new instance which will store its snapshots in the given directory.
    fn new(snapshot_dir: String) -> Self;
}
