//! The Raft storage interface and data types.

pub(crate) mod adapter;
mod callback;
mod helper;
mod log_store_ext;
mod snapshot_signature;
mod v2;

use std::fmt::Debug;
use std::ops::RangeBounds;

pub use adapter::Adaptor;
use async_trait::async_trait;
pub use helper::StorageHelper;
pub use log_store_ext::RaftLogReaderExt;
pub use snapshot_signature::SnapshotSignature;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;
pub use v2::RaftLogStorage;
pub use v2::RaftStateMachine;

use crate::node::Node;
use crate::raft_types::SnapshotId;
pub use crate::storage::callback::LogApplied;
pub use crate::storage::callback::LogFlushed;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StoredMembership;
use crate::Vote;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct SnapshotMeta<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Log entries upto which this snapshot includes, inclusive.
    pub last_log_id: Option<LogId<NID>>,

    /// The last applied membership config.
    pub last_membership: StoredMembership<NID, N>,

    /// To identify a snapshot when transferring.
    /// Caveat: even when two snapshot is built with the same `last_log_id`, they still could be
    /// different in bytes.
    pub snapshot_id: SnapshotId,
}

impl<NID, N> MessageSummary<SnapshotMeta<NID, N>> for SnapshotMeta<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn summary(&self) -> String {
        format!(
            "{{snapshot_id: {}, last_membership: {}, last_log_id: {}}}",
            self.snapshot_id,
            self.last_log_id.summary(),
            self.last_membership.summary()
        )
    }
}

impl<NID, N> SnapshotMeta<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub fn signature(&self) -> SnapshotSignature<NID> {
        SnapshotSignature {
            last_log_id: self.last_log_id,
            last_membership_log_id: *self.last_membership.log_id(),
            snapshot_id: self.snapshot_id.clone(),
        }
    }

    /// Returns a ref to the id of the last log that is included in this snasphot.
    pub fn last_log_id(&self) -> Option<&LogId<NID>> {
        self.last_log_id.as_ref()
    }
}

/// The data associated with the current snapshot.
#[derive(Debug)]
pub struct Snapshot<NID, N, SD>
where
    NID: NodeId,
    N: Node,
    SD: AsyncRead + AsyncSeek + Send + Unpin + 'static,
{
    /// metadata of a snapshot
    pub meta: SnapshotMeta<NID, N>,

    /// A read handle to the associated snapshot.
    pub snapshot: Box<SD>,
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
    /// Returns the last deleted log id and the last log id.
    ///
    /// The impl should not consider the applied log id in state machine.
    /// The returned `last_log_id` could be the log id of the last present log entry, or the
    /// `last_purged_log_id` if there is no entry at all.
    // NOTE: This can be made into sync, provided all state machines will use atomic read or the
    // like.
    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<C::NodeId>>;

    /// Get a series of log entries from storage.
    ///
    /// The start value is inclusive in the search and the stop value is non-inclusive: `[start,
    /// stop)`.
    ///
    /// Entry that is not found is allowed.
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> Result<Vec<C::Entry>, StorageError<C::NodeId>>;
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
    /// - Performing log compaction, e.g. merge log entries that operates on the same key, like a
    ///   LSM-tree does,
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

    /// To ensure correctness: the vote must be persisted on disk before returning.
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
    ///
    /// To ensure correctness:
    ///
    /// - All entries must be persisted on disk before returning.
    ///
    /// - There must not be a **hole** in logs. Because Raft only examine the last log id to ensure
    ///   correctness.
    async fn append_to_log<I>(&mut self, entries: I) -> Result<(), StorageError<C::NodeId>>
    where I: IntoIterator<Item = C::Entry> + Send;

    /// Delete conflict log entries since `log_id`, inclusive.
    ///
    /// This method is called by a follower or learner when the local logs conflict with the
    /// leaders.
    ///
    /// To ensure correctness:
    ///
    /// - When this function returns, the deleted logs must not be read(e.g., by
    ///   `RaftLogReader::try_get_log_entries()`) any more.
    ///
    /// - It must not leave a **hole** in the log. In other words, if it has to delete logs in more
    ///   than one transactions, it must delete logs in backward order. So that in a case server
    ///   crashes, it won't leave a hole.
    async fn delete_conflict_logs_since(&mut self, log_id: LogId<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;

    /// Delete applied log entries upto `log_id`, inclusive.
    ///
    /// To ensure correctness:
    ///
    /// - It must not leave a **hole** in logs.
    async fn purge_logs_upto(&mut self, log_id: LogId<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;

    // --- State Machine

    // TODO: This can be made into sync, provided all state machines will use atomic read or the
    //       like.
    // ---
    /// Returns the last applied log id which is recorded in state machine, and the last applied
    /// membership config.
    ///
    /// ## Correctness requirements
    ///
    /// It is all right to return a membership with greater log id than the
    /// last-applied-log-id.
    async fn last_applied_state(
        &mut self,
    ) -> Result<(Option<LogId<C::NodeId>>, StoredMembership<C::NodeId, C::Node>), StorageError<C::NodeId>>;

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
    /// For every entry to apply, an implementation should:
    /// - Store the log id as last applied log id.
    /// - Deal with the EntryPayload::Normal() log, which is business logic log.
    /// - Store membership config in EntryPayload::Membership.
    ///
    /// Note that for a membership log, the implementation need to do nothing about it, except
    /// storing it.
    ///
    /// An implementation may choose to persist either the state machine or the snapshot:
    ///
    /// - An implementation with persistent state machine: persists the state on disk before
    ///   returning from `apply_to_state_machine()`. So that a snapshot does not need to be
    ///   persistent.
    ///
    /// - An implementation with persistent snapshot: `apply_to_state_machine()` does not have to
    ///   persist state on disk. But every snapshot has to be persistent. And when starting up the
    ///   application, the state machine should be rebuilt from the last snapshot.
    async fn apply_to_state_machine(&mut self, entries: &[C::Entry]) -> Result<Vec<C::R>, StorageError<C::NodeId>>;

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

    /// Install a snapshot which has finished streaming from the leader.
    ///
    /// All other snapshots should be deleted at this point.
    ///
    /// ### snapshot
    /// A snapshot created from an earlier call to `begin_receiving_snapshot` which provided the
    /// snapshot.
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<C::NodeId, C::Node>,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<(), StorageError<C::NodeId>>;

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
