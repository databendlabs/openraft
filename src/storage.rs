//! A module encapsulating the Raft storage interface.

use actix::{
    dev::ToEnvelope,
    prelude::*,
};
use futures::sync::mpsc::UnboundedReceiver;
use failure::Fail;

use crate::{
    proto,
    raft::NodeId,
};

/// An error type which wraps a `dyn Fail` type coming from the storage layer.
///
/// This does require an allocation; however, the Raft node is currently configured to stop when
/// it encounteres an error from the storage layer. The cost of this allocation then is quite low.
///
/// In order to avoid potential data corruption or other such issues, when an error is observed
/// from the storage layer, the Raft node will stop. The parent application will still be able to
/// perform cleanup or any other routines as needed before shutdown.
#[derive(Debug, Fail)]
#[fail(display="{}", _0)]
pub struct StorageError(pub Box<dyn Fail>);

/// The result type of all `RaftStorage` interfaces.
pub type StorageResult<T> = Result<T, StorageError>;

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
pub struct GetInitialState;

impl Message for GetInitialState {
    type Result = StorageResult<InitialState>;
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
pub struct GetLogEntries {
    pub start: u64,
    pub stop: u64,
}

impl Message for GetLogEntries {
    type Result = StorageResult<Vec<proto::Entry>>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// AppendLogEntries //////////////////////////////////////////////////////////////////////////////

/// An actix message type for requesting a series of entries to be written to the log.
///
/// Though the entries will always be presented in order, each entry's index should be used for
/// determining its location to be written in the log, as logs may need to be overwritten under
/// some circumstances.
///
/// The result of a successful append entries call must contain the details on that last log entry
/// appended to the log.
pub struct AppendLogEntries(pub Vec<proto::Entry>);

/// Details on the last log entry appended to the log as part of an `AppendLogEntries` operation.
pub struct AppendLogEntriesData {
    pub index: u64,
    pub term: u64,
}

impl Message for AppendLogEntries {
    type Result = StorageResult<AppendLogEntriesData>;
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
/// `through`. The entry at index `through` should be replaced with a new entry created from
/// calling `actix_raft::proto::Entry::new_snapshot_pointer(...)`.
/// - Any old snapshot will no longer have representation in the log, and should be deleted.
/// - Return a copy of the snapshot pointer entry created earlier.
pub struct CreateSnapshot {
    /// The new snapshot should start from entry `0` and should cover all entries through the
    /// index specified here, inclusive.
    through: u64,
    /// The directory where the new snapshot is to be written.
    snapshot_dir: String,
}

impl Message for CreateSnapshot {
    type Result = StorageResult<proto::Entry>;
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
/// `offset` specified in the chunk. Due to transient communications issues, chunks may be
/// redelivered.
/// - If the receiver is dropped, the snapshot which was being created should be removed from
/// disk.
///
/// Once a chunk is received which is the final chunk of the snapshot, after writing the data,
/// there are a few important steps to take:
///
/// - Create a new entry in the log via the `actix_raft::proto::Entry::new_snapshot_pointer(...)`
/// constructor. Insert the new entry into the log at the specified `index` of this payload.
/// - If there are any logs older than `index`, remove them.
/// - If there are any other snapshots in `snapshot_dir`, remove them.
/// - If there are any logs newer than `index`, then return.
/// - If there are no logs newer than `index`, then the state machine should be reset, and
/// recreated from the new snapshot. Return once the state machine has been brought up-to-date.
pub struct InstallSnapshot {
    /// The term which the final entry of this snapshot covers.
    term: u64,
    /// The index of the final entry which this snapshot covers.
    index: u64,
    /// The directory where the new snapshot is to be written.
    snapshot_dir: String,
    /// A stream of data chunks for this snapshot.
    stream: UnboundedReceiver<InstallSnapshotChunk>,
}

impl Message for InstallSnapshot {
    type Result = StorageResult<()>;
}

/// A chunk of snapshot data.
pub struct InstallSnapshotChunk {
    /// The byte offset where chunk is positioned in the snapshot file.
    offset: u64,
    /// The raw bytes of the snapshot chunk, starting at `offset`.
    data: Vec<u8>,
    /// Will be `true` if this is the last chunk in the snapshot.
    done: bool,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// GetCurrentSnapshot ////////////////////////////////////////////////////////////////////////////

/// A request from the Raft node to get the location of the current snapshot on disk.
///
/// ### implementation algorithm
/// Implementation for this type's handler should be quite simple. Check the directory specified
/// by `snapshot_dir` for any snapshot files. A proper implementation will only ever have one
/// active snapshot, though another may exist while it is being created. As such, it is
/// recommended to use a file naming pattern which will allow for easily distinguishing betweeen
/// the current live snapshot, and any new snapshot which is being created.
///
/// Once the current snapshot has been located, the absolute path to the file should be returned.
/// If there is no active snapshot file, then `None` should be returned.
pub struct GetCurrentSnapshot {
    /// The directory where the system has been configured to store snapshots.
    snapshot_dir: String,
}

impl Message for GetCurrentSnapshot {
    type Result = StorageResult<Option<String>>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// GetCurrentSnapshot ////////////////////////////////////////////////////////////////////////////

/// A request from the Raft node to apply the given log entries to the state machine.
///
/// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
/// have been replicated to a majority of the cluster, will be applied to the state machine.
pub struct ApplyEntriesToStateMachine(pub Vec<proto::Entry>);

/// Details on the last log entry applied to the state machine as part of an `ApplyEntriesToStateMachine` operation.
pub struct ApplyEntriesToStateMachineData {
    pub index: u64,
    pub term: u64,
}

impl Message for ApplyEntriesToStateMachine {
    type Result = StorageResult<ApplyEntriesToStateMachineData>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// SaveHardState /////////////////////////////////////////////////////////////////////////////////

/// A request from the Raft node to save its HardState.
pub struct SaveHardState(pub HardState);

/// A record holding the hard state of a Raft node.
pub struct HardState {
    /// The last recorded term observed by this system.
    pub current_term: u64,
    /// The ID of the node voted for in the `current_term`.
    pub voted_for: Option<NodeId>,
    /// The IDs of all known members of the cluster.
    pub members: Vec<u64>,
}

impl Message for SaveHardState {
    type Result = StorageResult<()>;
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
/// Log compaction, which is what taking a snapshot is for, is an application specific process.
/// The essential idea is that superfluous records in the log will be removed. See §7 for more
/// details. There are a few snapshot related messages which the `RaftStorage` actor must handle.
///
/// - `CreateSnapshot`: a request to create a new snapshot of the current log.
/// - `InstallSnapshot`: the Raft leader is streaming over a snapshot, install it.
/// - `GetCurrentSnapshot`: the Raft node needs to know the location of the current snapshot.
///
/// See each message type for more details on the message and how to properly implement their
/// behaviors.
pub trait RaftStorage
    where
        Self: Actor<Context=Context<Self>>,
        Self: Handler<GetInitialState> + ToEnvelope<Self, GetInitialState>,
        Self: Handler<GetLogEntries> + ToEnvelope<Self, GetLogEntries>,
        Self: Handler<AppendLogEntries> + ToEnvelope<Self, AppendLogEntries>,
        Self: Handler<ApplyEntriesToStateMachine> + ToEnvelope<Self, ApplyEntriesToStateMachine>,
        Self: Handler<CreateSnapshot> + ToEnvelope<Self, CreateSnapshot>,
        Self: Handler<InstallSnapshot> + ToEnvelope<Self, InstallSnapshot>,
        Self: Handler<GetCurrentSnapshot> + ToEnvelope<Self, GetCurrentSnapshot>,
        Self: Handler<SaveHardState> + ToEnvelope<Self, SaveHardState>,
{}
