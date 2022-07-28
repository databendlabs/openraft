//! The Raft storage interface and data types.

mod helper;
mod snapshot_signature;
use std::fmt::Debug;
use std::ops::RangeBounds;

use async_trait::async_trait;
pub use helper::StorageHelper;
pub use snapshot_signature::SnapshotSignature;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;

use crate::defensive::check_range_matches_entries;
use crate::membership::EffectiveMembership;
use crate::node::Node;
use crate::raft_types::SnapshotId;
use crate::raft_types::StateMachineChanges;
use crate::Entry;
use crate::LogId;
use crate::NodeId;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Vote;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct SnapshotMeta<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Log entries upto which this snapshot includes, inclusive.
    pub last_log_id: LogId<NID>,

    /// The last applied membership config.
    pub last_membership: EffectiveMembership<NID, N>,

    /// To identify a snapshot when transferring.
    /// Caveat: even when two snapshot is built with the same `last_log_id`, they still could be different in bytes.
    pub snapshot_id: SnapshotId,
}

impl<NID, N> SnapshotMeta<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub fn signature(&self) -> SnapshotSignature<NID> {
        SnapshotSignature {
            last_log_id: Some(self.last_log_id),
            last_membership_log_id: self.last_membership.log_id,
            snapshot_id: self.snapshot_id.clone(),
        }
    }
}

/// The data associated with the current snapshot.
#[derive(Debug)]
pub struct Snapshot<NID, N, S>
where
    NID: NodeId,
    N: Node,
    S: AsyncRead + AsyncSeek + Send + Unpin + 'static,
{
    /// metadata of a snapshot
    pub meta: SnapshotMeta<NID, N>,

    /// A read handle to the associated snapshot.
    pub snapshot: Box<S>,
}

/// The state about logs.
///
/// Invariance: last_purged_log_id <= last_applied <= last_log_id
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogState<C: RaftTypeConfig> {
    /// The greatest log id that has been purged after being applied to state machine.
    pub last_purged_log_id: Option<LogId<C::NodeId>>,

    /// The log id of the last present entry if there are any entries.
    /// Otherwise the same value as `last_purged_log_id`.
    pub last_log_id: Option<LogId<C::NodeId>>,
}

/// A trait defining the interface for a Raft log subsystem.
///
/// This interface is accessed read-only from replica streams.
///
/// Typically, the log reader implementation as such will be hidden behind an `Arc<T>` and
/// this interface implemented on the `Arc<T>`. It can be co-implemented with [`RaftStorage`]
/// interface on the same cloneable object, if the underlying state machine is anyway synchronized.
#[async_trait]
pub trait RaftLogReader<C>: Send + Sync + 'static
where C: RaftTypeConfig
{
    /// Get a series of log entries from storage.
    ///
    /// Similar to `try_get_log_entries` except an error will be returned if there is an entry not
    /// found in the specified range.
    async fn get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<C>>, StorageError<C::NodeId>> {
        let res = self.try_get_log_entries(range.clone()).await?;

        check_range_matches_entries(range, &res)?;

        Ok(res)
    }

    /// Try to get an log entry.
    ///
    /// It does not return an error if the log entry at `log_index` is not found.
    async fn try_get_log_entry(&mut self, log_index: u64) -> Result<Option<Entry<C>>, StorageError<C::NodeId>> {
        let mut res = self.try_get_log_entries(log_index..(log_index + 1)).await?;
        Ok(res.pop())
    }

    /// Returns the last deleted log id and the last log id.
    ///
    /// The impl should not consider the applied log id in state machine.
    /// The returned `last_log_id` could be the log id of the last present log entry, or the `last_purged_log_id` if
    /// there is no entry at all.
    // NOTE: This can be made into sync, provided all state machines will use atomic read or the like.
    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<C::NodeId>>;

    /// Get a series of log entries from storage.
    ///
    /// The start value is inclusive in the search and the stop value is non-inclusive: `[start, stop)`.
    ///
    /// Entry that is not found is allowed.
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<C>>, StorageError<C::NodeId>>;
}

/// A trait defining the interface for a Raft state machine snapshot subsystem.
///
/// This interface is accessed read-only from snapshot building task.
///
/// Typically, the snapshot implementation as such will be hidden behind a reference type like
/// `Arc<T>` or `Box<T>` and this interface implemented on the reference type. It can be
/// co-implemented with [`RaftStorage`] interface on the same cloneable object, if the underlying
/// state machine is anyway synchronized.
#[async_trait]
pub trait RaftSnapshotBuilder<C, SD>: Send + Sync + 'static
where
    C: RaftTypeConfig,
    SD: AsyncRead + AsyncWrite + AsyncSeek + Send + Sync + Unpin + 'static,
{
    /// Build snapshot
    ///
    /// A snapshot has to contain information about exactly all logs up to the last applied.
    ///
    /// Building snapshot can be done by:
    /// - Performing log compaction, e.g. merge log entries that operates on the same key, like a LSM-tree does,
    /// - or by fetching a snapshot from the state machine.
    async fn build_snapshot(&mut self) -> Result<Snapshot<C::NodeId, C::Node, SD>, StorageError<C::NodeId>>;

    // NOTES:
    // This interface is geared toward small file-based snapshots. However, not all snapshots can
    // be easily represented as a file. Probably a more generic interface will be needed to address
    // also other needs.
}

/// A trait defining the interface for a Raft storage system.
///
/// See the [storage chapter of the guide](https://datafuselabs.github.io/openraft/storage.html)
/// for details and discussion on this trait and how to implement it.
///
/// Typically, the storage implementation as such will be hidden behind a `Box<T>`, `Arc<T>` or
/// a similar, more advanced reference type and this interface implemented on that reference type.
///
/// All methods on the storage are called inside of Raft core task. There is no concurrency on the
/// storage, except concurrency with snapshot builder and log reader, both created by this API.
/// The implementation of the API has to cope with (infrequent) concurrent access from these two
/// components.
#[async_trait]
pub trait RaftStorage<C>: RaftLogReader<C> + Send + Sync + 'static
where C: RaftTypeConfig
{
    /// The storage engine's associated type used for exposing a snapshot for reading & writing.
    ///
    /// See the [storage chapter of the guide](https://datafuselabs.github.io/openraft/getting-started.html#implement-raftstorage)
    /// for details on where and how this is used.
    type SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Send + Sync + Unpin + 'static;

    /// Log reader type.
    type LogReader: RaftLogReader<C>;

    /// Snapshot builder type.
    type SnapshotBuilder: RaftSnapshotBuilder<C, Self::SnapshotData>;

    // --- Vote

    async fn save_vote(&mut self, vote: &Vote<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;

    async fn read_vote(&mut self) -> Result<Option<Vote<C::NodeId>>, StorageError<C::NodeId>>;

    // --- Log

    /// Get the log reader.
    ///
    /// The method is intentionally async to give the implementation a chance to use asynchronous
    /// sync primitives to serialize access to the common internal object, if needed.
    async fn get_log_reader(&mut self) -> Self::LogReader;

    /// Append a payload of entries to the log.
    ///
    /// Though the entries will always be presented in order, each entry's index should be used to
    /// determine its location to be written in the log.
    async fn append_to_log(&mut self, entries: &[&Entry<C>]) -> Result<(), StorageError<C::NodeId>>;

    /// Delete conflict log entries since `log_id`, inclusive.
    async fn delete_conflict_logs_since(&mut self, log_id: LogId<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;

    /// Delete applied log entries upto `log_id`, inclusive.
    async fn purge_logs_upto(&mut self, log_id: LogId<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;

    // --- State Machine

    /// Returns the last applied log id which is recorded in state machine, and the last applied membership log id and
    /// membership config.
    // NOTE: This can be made into sync, provided all state machines will use atomic read or the like.
    async fn last_applied_state(
        &mut self,
    ) -> Result<(Option<LogId<C::NodeId>>, EffectiveMembership<C::NodeId, C::Node>), StorageError<C::NodeId>>;

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
    // TODO The reply should happen asynchronously, somehow. Make this method synchronous and
    // instead of using the result, pass a channel where to post the completion. The Raft core can
    // then collect completions on this channel and update the client with the result once all
    // the preceding operations have been applied to the state machine. This way we'll reach
    // operation pipelining w/o the need to wait for the completion of each operation inline.
    async fn apply_to_state_machine(&mut self, entries: &[&Entry<C>]) -> Result<Vec<C::R>, StorageError<C::NodeId>>;

    // --- Snapshot

    /// Get the snapshot builder for the state machine.
    ///
    /// The method is intentionally async to give the implementation a chance to use asynchronous
    /// sync primitives to serialize access to the common internal object, if needed.
    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder;

    /// Create a new blank snapshot, returning a writable handle to the snapshot object.
    ///
    /// Raft will use this handle to receive snapshot data.
    ///
    /// ### implementation guide
    /// See the [storage chapter of the guide](https://datafuselabs.github.io/openraft/storage.html)
    /// for details on log compaction / snapshotting.
    async fn begin_receiving_snapshot(&mut self) -> Result<Box<Self::SnapshotData>, StorageError<C::NodeId>>;

    /// Install a snapshot which has finished streaming from the cluster leader.
    ///
    /// All other snapshots should be deleted at this point.
    ///
    /// ### snapshot
    /// A snapshot created from an earlier call to `begin_receiving_snapshot` which provided the snapshot.
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<C::NodeId, C::Node>,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<StateMachineChanges<C>, StorageError<C::NodeId>>;

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
    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<C::NodeId, C::Node, Self::SnapshotData>>, StorageError<C::NodeId>>;
}

/// APIs for debugging a store.
#[async_trait]
pub trait RaftStorageDebug<SM> {
    /// Get a handle to the state machine for testing purposes.
    async fn get_state_machine(&mut self) -> SM;
}
