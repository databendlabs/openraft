//! The Raft storage interface and data types.

use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite};

use crate::{AppData, AppDataResponse, AppError, NodeId};
use crate::raft::{Entry, MembershipConfig};

/// The data associated with the current snapshot.
pub struct CurrentSnapshotData<S>
    where S: AsyncRead + AsyncSeek + Send + Unpin + 'static,
{
    /// The snapshot entry's term.
    pub term: u64,
    /// The snapshot entry's index.
    pub index: u64,
    /// The latest membership configuration covered by the snapshot.
    pub membership: MembershipConfig,
    /// A read handle to the associated snapshot.
    pub snapshot: Box<S>,
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
    pub membership: MembershipConfig,
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

impl InitialState {
    /// Create a new instance for a pristine Raft node.
    ///
    /// ### `id`
    /// The ID of the Raft node.
    pub fn new_initial(id: NodeId) -> Self {
        Self{last_log_index: 0, last_log_term: 0, last_applied_log: 0, hard_state: HardState{
            current_term: 0, voted_for: None, membership: MembershipConfig::new_initial(id),
        }}
    }
}

/// A trait defining the interface for a Raft storage system.
///
/// See the [storage chapter of the guide](TODO:)
/// for details and discussion on this trait and how to implement it.
#[async_trait]
pub trait RaftStorage<D, R, E>: Send + Sync + 'static
    where
        D: AppData,
        R: AppDataResponse,
        E: AppError,
{
    /// The storage engine's associated type used for exposing a snapshot for reading & writing.
    type Snapshot: AsyncRead + AsyncWrite + AsyncSeek + Send + Unpin + 'static;

    /// A request from Raft to get Raft's state information from storage.
    ///
    /// When the Raft node is first started, it will call this interface on the storage system to
    /// fetch the last known state from stable storage. If no such entry exists due to being the
    /// first time the node has come online, then `InitialState::new_initial` should be used.
    ///
    /// ### pro tip
    /// The storage impl may need to look in a few different places to accurately respond to this
    /// request: the last entry in the log for `last_log_index` & `last_log_term`; the node's hard
    /// state record; and the index of the last log applied to the state machine.
    async fn get_initial_state(&self) -> Result<InitialState, E>;

    /// A request from Raft to save its hard state.
    async fn save_hard_state(&self, hs: &HardState) -> Result<(), E>;

    /// A request from Raft to get a series of log entries from storage.
    ///
    /// The start value is inclusive in the search and the stop value is non-inclusive: `[start, stop)`.
    async fn get_log_entries(&self, start: u64, stop: u64) -> Result<Vec<Entry<D>>, E>;

    /// Delete all logs starting from `start` and stopping at `stop`, else continuing to the end
    /// of the log if `stop` is `None`.
    async fn delete_logs_from(&self, start: u64, stop: Option<u64>) -> Result<(), E>;

    /// A request from Raft to append a new entry to the log.
    ///
    /// These requests come about via client requests, and as such, this is the only stroage method
    /// which is allowed to return errors which will not cause Raft to shutdown. Application
    /// errors coming from this interface will be sent back to the client as-is.
    ///
    /// This property of error handling allows you to keep your application logic as close to the
    /// storage layer as needed.
    async fn append_entry_to_log(&self, entry: &Entry<D>) -> Result<(), E>;

    /// A request from Raft to replicate a payload of entries to the log.
    ///
    /// These requests come about via the Raft leader's replication process. An error coming from this
    /// interface will cause Raft to shutdown, as this is not where application logic should be
    /// returning application specific errors. Application specific constraints may only be enforced
    /// in the `AppendEntryToLog` handler.
    ///
    /// Though the entries will always be presented in order, each entry's index should be used to
    /// determine its location to be written in the log, as logs may need to be overwritten under
    /// some circumstances.
    async fn replicate_to_log(&self, entries: &[Entry<D>]) -> Result<(), E>;

    /// A request from Raft to apply the given log entry to the state machine.
    ///
    /// This handler is called as part of the client request path. Client requests which are
    /// configured to respond after they have been `Applied` will wait until after this handler
    /// returns before issuing a response to the client request.
    ///
    /// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
    /// have been replicated to a majority of the cluster, will be applied to the state machine.
    async fn apply_entry_to_state_machine(&self, entry: &Entry<D>) -> Result<R, E>;

    /// A request from Raft to apply the given payload of entries to the state machine, as part of replication.
    ///
    /// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
    /// have been replicated to a majority of the cluster, will be applied to the state machine.
    async fn replicate_to_state_machine(&self, entries: &[Entry<D>]) -> Result<(), E>;

    /// A request from Raft to perform log compaction, returning a handle to the generated snapshot.
    ///
    /// ### `through`
    /// The log should be compacted starting from entry `0` and should cover all entries through the
    /// index specified by `through`, inclusively. This will always be the `commit_index` of the
    /// Raft log at the time of the request.
    ///
    /// ### implementation guide
    /// See the [storage chapter of the guide](TODO:)
    /// for details on how to implement this handler.
    async fn do_log_compaction(&self, through: u64) -> Result<CurrentSnapshotData<Self::Snapshot>, E>;

    /// Create a new snapshot returning a writable handle to the snapshot object along with the ID of the snapshot.
    ///
    /// ### implementation guide
    /// See the [storage chapter of the guide]()
    /// for details on how to implement this handler.
    async fn create_snapshot(&self) -> Result<(String, Box<Self::Snapshot>), E>;

    /// Finalize the installation of a snapshot which has finished streaming from the cluster leader.
    ///
    /// Delete all entries in the log, stopping at `delete_through`, unless `None`, in which case
    /// all entries of the log are to be deleted.
    ///
    /// Write a new snapshot pointer to the log at the given `index`. The snapshot pointer should be
    /// constructed via the `Entry::new_snapshot_pointer` constructor.
    ///
    /// All other snapshots should be deleted at this point.
    ///
    /// ### snapshot
    /// A snapshot created from an earlier call to `created_snapshot` which provided the snapshot.
    /// By the time ownership of the snapshot object is returned here, its
    /// `AsyncWriteExt.shutdown()` method will have been called, so no additional writes should be
    /// made to the snapshot.
    async fn finalize_snapshot_installation(
        &self, index: u64, term: u64, delete_through: Option<u64>,
        id: String, snapshot: Box<Self::Snapshot>,
    ) -> Result<(), E>;

    /// A request from Raft to get a readable handle to the current snapshot, along with its metadata.
    ///
    /// ### implementation algorithm
    /// Implementing this method should be straightforward. Check the configured snapshot
    /// directory for any snapshot files. A proper implementation will only ever have one
    /// active snapshot, though another may exist while it is being created. As such, it is
    /// recommended to use a file naming pattern which will allow for easily distinguishing between
    /// the current live snapshot, and any new snapshot which is being created.
    ///
    /// A proper snapshot implementation will store the term, index and membership config as part
    /// of the snapshot as well, which can be decoded for creating this method's response.
    async fn get_current_snapshot(&self) -> Result<Option<CurrentSnapshotData<Self::Snapshot>>, E>;
}
