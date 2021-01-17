//! The Raft storage interface and data types.

use std::error::Error;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite};

use crate::raft::{Entry, MembershipConfig};
use crate::{AppData, AppDataResponse, NodeId};

/// The data associated with the current snapshot.
pub struct CurrentSnapshotData<S>
where
    S: AsyncRead + AsyncSeek + Send + Unpin + 'static,
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
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct HardState {
    /// The last recorded term observed by this system.
    pub current_term: u64,
    /// The ID of the node voted for in the `current_term`.
    pub voted_for: Option<NodeId>,
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
    /// The latest cluster membership configuration found in the log, else a new initial
    /// membership config consisting only of this node's ID.
    pub membership: MembershipConfig,
}

impl InitialState {
    /// Create a new instance for a pristine Raft node.
    ///
    /// ### `id`
    /// The ID of the Raft node.
    pub fn new_initial(id: NodeId) -> Self {
        Self {
            last_log_index: 0,
            last_log_term: 0,
            last_applied_log: 0,
            hard_state: HardState {
                current_term: 0,
                voted_for: None,
            },
            membership: MembershipConfig::new_initial(id),
        }
    }
}

/// A trait defining the interface for a Raft storage system.
///
/// See the [storage chapter of the guide](https://async-raft.github.io/async-raft/storage.html)
/// for details and discussion on this trait and how to implement it.
#[async_trait]
pub trait RaftStorage<D, R>: Send + Sync + 'static
where
    D: AppData,
    R: AppDataResponse,
{
    /// The storage engine's associated type used for exposing a snapshot for reading & writing.
    type Snapshot: AsyncRead + AsyncWrite + AsyncSeek + Send + Unpin + 'static;
    /// The error type used to indicate to Raft that shutdown is needed when calling the
    /// `apply_entry_to_state_machine` method.
    ///
    /// This error type is only considered for the `apply_entry_to_state_machine` method as it is
    /// the only method which is allowed to return errors normally as part of application logic.
    ///
    /// For all other methods of this trait, returning an error will cause Raft to shutdown.
    type ShutdownError: Error + Send + Sync + 'static;

    /// Get the latest membership config found in the log.
    ///
    /// This must always be implemented as a reverse search through the log to find the most
    /// recent membership config to be appended to the log.
    ///
    /// If a snapshot pointer is encountered, then the membership config embedded in that snapshot
    /// pointer should be used.
    ///
    /// If the system is pristine, then it should return the value of calling
    /// `MembershipConfig::new_initial(node_id)`. It is required that the storage engine persist
    /// the node's ID so that it is consistent across restarts.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn get_membership_config(&self) -> Result<MembershipConfig>;

    /// Get Raft's state information from storage.
    ///
    /// When the Raft node is first started, it will call this interface on the storage system to
    /// fetch the last known state from stable storage. If no such entry exists due to being the
    /// first time the node has come online, then `InitialState::new_initial` should be used.
    ///
    /// **Pro tip:** the storage impl may need to look in a few different places to accurately
    /// respond to this request: the last entry in the log for `last_log_index` & `last_log_term`;
    /// the node's hard state record; and the index of the last log applied to the state machine.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn get_initial_state(&self) -> Result<InitialState>;

    /// Save Raft's hard-state.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn save_hard_state(&self, hs: &HardState) -> Result<()>;

    /// Get a series of log entries from storage.
    ///
    /// The start value is inclusive in the search and the stop value is non-inclusive: `[start, stop)`.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn get_log_entries(&self, start: u64, stop: u64) -> Result<Vec<Entry<D>>>;

    /// Delete all logs starting from `start` and stopping at `stop`, else continuing to the end
    /// of the log if `stop` is `None`.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn delete_logs_from(&self, start: u64, stop: Option<u64>) -> Result<()>;

    /// Append a new entry to the log.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn append_entry_to_log(&self, entry: &Entry<D>) -> Result<()>;

    /// Replicate a payload of entries to the log.
    ///
    /// Though the entries will always be presented in order, each entry's index should be used to
    /// determine its location to be written in the log.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn replicate_to_log(&self, entries: &[Entry<D>]) -> Result<()>;

    /// Apply the given log entry to the state machine.
    ///
    /// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
    /// have been replicated to a majority of the cluster, will be applied to the state machine.
    ///
    /// This is where the business logic of interacting with your application's state machine
    /// should live. This is 100% application specific. Perhaps this is where an application
    /// specific transaction is being started, or perhaps committed. This may be where a key/value
    /// is being stored. This may be where an entry is being appended to an immutable log.
    ///
    /// Error handling for this method is note worthy. If an error is returned from a call to this
    /// method, the error will be inspected, and if the error is an instance of
    /// `RaftStorage::ShutdownError`, then Raft will go into shutdown in order to preserve the
    /// safety of the data and avoid corruption. Any other errors will be propagated back up to the
    /// `Raft.client_write` call point.
    ///
    /// It is important to note that even in cases where an application specific error is returned,
    /// implementations should still record that the entry has been applied to the state machine.
    async fn apply_entry_to_state_machine(&self, index: &u64, data: &D) -> Result<R>;

    /// Apply the given payload of entries to the state machine, as part of replication.
    ///
    /// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
    /// have been replicated to a majority of the cluster, will be applied to the state machine.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn replicate_to_state_machine(&self, entries: &[(&u64, &D)]) -> Result<()>;

    /// Perform log compaction, returning a handle to the generated snapshot.
    ///
    /// ### implementation guide
    /// When performing log compaction, the compaction can only cover the breadth of the log up to
    /// the last applied log and under write load this value may change quickly. As such, the
    /// storage implementation should export/checkpoint/snapshot its state machine, and then use
    /// the value of that export's last applied log as the metadata indicating the breadth of the
    /// log covered by the snapshot.
    ///
    /// Errors returned from this method will be logged and retried.
    async fn do_log_compaction(&self) -> Result<CurrentSnapshotData<Self::Snapshot>>;

    /// Create a new blank snapshot, returning a writable handle to the snapshot object along with
    /// the ID of the snapshot.
    ///
    /// ### implementation guide
    /// See the [storage chapter of the guide](https://async-raft.github.io/async-raft/storage.html)
    /// for details on log compaction / snapshotting.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn create_snapshot(&self) -> Result<(String, Box<Self::Snapshot>)>;

    /// Finalize the installation of a snapshot which has finished streaming from the cluster leader.
    ///
    /// Delete all entries in the log through `delete_through`, unless `None`, in which case
    /// all entries of the log are to be deleted.
    ///
    /// Write a new snapshot pointer to the log at the given `index`. The snapshot pointer should be
    /// constructed via the `Entry::new_snapshot_pointer` constructor and the other parameters
    /// provided to this method.
    ///
    /// All other snapshots should be deleted at this point.
    ///
    /// ### snapshot
    /// A snapshot created from an earlier call to `created_snapshot` which provided the snapshot.
    /// By the time ownership of the snapshot object is returned here, its
    /// `AsyncWriteExt.shutdown()` method will have been called, so no additional writes should be
    /// made to the snapshot.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn finalize_snapshot_installation(
        &self, index: u64, term: u64, delete_through: Option<u64>, id: String, snapshot: Box<Self::Snapshot>,
    ) -> Result<()>;

    /// Get a readable handle to the current snapshot, along with its metadata.
    ///
    /// ### implementation algorithm
    /// Implementing this method should be straightforward. Check the configured snapshot
    /// directory for any snapshot files. A proper implementation will only ever have one
    /// active snapshot, though another may exist while it is being created. As such, it is
    /// recommended to use a file naming pattern which will allow for easily distinguishing between
    /// the current live snapshot, and any new snapshot which is being created.
    ///
    /// A proper snapshot implementation will store the term, index and membership config as part
    /// of the snapshot, which should be decoded for creating this method's response data.
    ///
    /// Errors returned from this method will cause Raft to go into shutdown.
    async fn get_current_snapshot(&self) -> Result<Option<CurrentSnapshotData<Self::Snapshot>>>;
}
