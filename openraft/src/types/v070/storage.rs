//! The Raft storage interface and data types.

use std::fmt::Debug;
use std::ops::RangeBounds;

use async_trait::async_trait;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;

use super::log_state::LogState;
use super::AppData;
use super::AppDataResponse;
use super::EffectiveMembership;
use super::Entry;
use super::HardState;
use super::LogId;
use super::Snapshot;
use super::SnapshotMeta;
use super::StateMachineChanges;
use super::StorageError;

/// A trait defining the interface for a Raft storage system.
///
/// See the [storage chapter of the guide](https://datafuselabs.github.io/openraft/storage.html)
/// for details and discussion on this trait and how to implement it.
#[async_trait]
pub trait RaftStorage<D, R>: Send + Sync + 'static
where
    D: AppData,
    R: AppDataResponse,
{
    /// The storage engine's associated type used for exposing a snapshot for reading & writing.
    ///
    /// See the [storage chapter of the guide](https://datafuselabs.github.io/openraft/getting-started.html#implement-raftstorage)
    /// for details on where and how this is used.
    type SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Send + Sync + Unpin + 'static;

    // --- Hard State

    async fn save_hard_state(&self, hs: &HardState) -> Result<(), StorageError>;

    async fn read_hard_state(&self) -> Result<Option<HardState>, StorageError>;

    // --- Log

    /// Returns the last deleted log id and the last log id.
    ///
    /// The impl should not consider the applied log id in state machine.
    /// The returned `last_log_id` could be the log id of the last present log entry, or the `last_purged_log_id` if
    /// there is no entry at all.
    async fn get_log_state(&self) -> Result<LogState, StorageError>;

    /// Get a series of log entries from storage.
    ///
    /// The start value is inclusive in the search and the stop value is non-inclusive: `[start, stop)`.
    ///
    /// Entry that is not found is allowed.
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &self,
        range: RB,
    ) -> Result<Vec<Entry<D>>, StorageError>;

    /// Append a payload of entries to the log.
    ///
    /// Though the entries will always be presented in order, each entry's index should be used to
    /// determine its location to be written in the log.
    async fn append_to_log(&self, entries: &[&Entry<D>]) -> Result<(), StorageError>;

    /// Delete conflict log entries since `log_id`, inclusive.
    async fn delete_conflict_logs_since(&self, log_id: LogId) -> Result<(), StorageError>;

    /// Delete applied log entries upto `log_id`, inclusive.
    async fn purge_logs_upto(&self, log_id: LogId) -> Result<(), StorageError>;

    // --- State Machine

    /// Returns the last applied log id which is recorded in state machine, and the last applied membership log id and
    /// membership config.
    async fn last_applied_state(&self) -> Result<(Option<LogId>, Option<EffectiveMembership>), StorageError>;

    /// Apply the given payload of entries to the state machine.
    ///
    /// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
    /// have been replicated to a quorum of the cluster, will be applied to the state machine.
    ///
    /// This is where the business logic of interacting with your application's state machine
    /// should live. This is 100% application specific. Perhaps this is where an application
    /// specific transaction is being started, or perhaps committed. This may be where a key/value
    /// is being stored.
    ///
    /// An impl should do:
    /// - Store the last applied log id.
    /// - Deal with the EntryPayload::Normal() log, which is business logic log.
    /// - Deal with EntryPayload::Membership, store the membership config.
    async fn apply_to_state_machine(&self, entries: &[&Entry<D>]) -> Result<Vec<R>, StorageError>;

    // --- Snapshot

    /// Build snapshot
    ///
    /// A snapshot has to contain information about exactly all logs upto the last applied.
    ///
    /// Building snapshot can be done by:
    /// - Performing log compaction, e.g. merge log entries that operates on the same key, like a LSM-tree does,
    /// - or by fetching a snapshot from the state machine.
    async fn build_snapshot(&self) -> Result<Snapshot<Self::SnapshotData>, StorageError>;

    /// Create a new blank snapshot, returning a writable handle to the snapshot object.
    ///
    /// Raft will use this handle to receive snapshot data.
    ///
    /// ### implementation guide
    /// See the [storage chapter of the guide](https://datafuselabs.github.io/openraft/storage.html)
    /// for details on log compaction / snapshotting.
    async fn begin_receiving_snapshot(&self) -> Result<Box<Self::SnapshotData>, StorageError>;

    /// Install a snapshot which has finished streaming from the cluster leader.
    ///
    /// All other snapshots should be deleted at this point.
    ///
    /// ### snapshot
    /// A snapshot created from an earlier call to `begin_receiving_snapshot` which provided the snapshot.
    async fn install_snapshot(
        &self,
        meta: &SnapshotMeta,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<StateMachineChanges, StorageError>;

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
    async fn get_current_snapshot(&self) -> Result<Option<Snapshot<Self::SnapshotData>>, StorageError>;
}

/// APIs for debugging a store.
#[async_trait]
pub trait RaftStorageDebug<SM> {
    /// Get a handle to the state machine for testing purposes.
    async fn get_state_machine(&self) -> SM;
}
