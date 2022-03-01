//! The Raft storage interface and data types.

use std::fmt::Debug;
use std::ops::RangeBounds;

use async_trait::async_trait;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;

use crate::core::EffectiveMembership;
use crate::defensive::check_range_matches_entries;
use crate::raft::Entry;
use crate::raft::EntryPayload;
use crate::raft_types::SnapshotId;
use crate::raft_types::StateMachineChanges;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Vote;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta<C: RaftTypeConfig> {
    // Log entries upto which this snapshot includes, inclusive.
    pub last_log_id: LogId<C>,

    /// To identify a snapshot when transferring.
    /// Caveat: even when two snapshot is built with the same `last_log_id`, they still could be different in bytes.
    pub snapshot_id: SnapshotId,
}

/// The data associated with the current snapshot.
#[derive(Debug)]
pub struct Snapshot<C, S>
where
    C: RaftTypeConfig,
    S: AsyncRead + AsyncSeek + Send + Unpin + 'static,
{
    /// metadata of a snapshot
    pub meta: SnapshotMeta<C>,

    /// A read handle to the associated snapshot.
    pub snapshot: Box<S>,
}

/// A struct used to represent the initial state which a Raft node needs when first starting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InitialState<C: RaftTypeConfig> {
    /// The last entry.
    pub last_log_id: Option<LogId<C>>,

    /// The LogId of the last log applied to the state machine.
    pub last_applied: Option<LogId<C>>,

    pub vote: Vote<C>,

    /// The latest cluster membership configuration found, in log or in state machine, else a new initial
    /// membership config consisting only of this node's ID.
    pub last_membership: Option<EffectiveMembership<C>>,
}

/// The state about logs.
///
/// Invariance: last_purged_log_id <= last_applied <= last_log_id
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogState<C: RaftTypeConfig> {
    /// The greatest log id that has been purged after being applied to state machine.
    pub last_purged_log_id: Option<LogId<C>>,

    /// The log id of the last present entry if there are any entries.
    /// Otherwise the same value as `last_purged_log_id`.
    pub last_log_id: Option<LogId<C>>,
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
    ) -> Result<Vec<Entry<C>>, StorageError<C>> {
        let res = self.try_get_log_entries(range.clone()).await?;

        check_range_matches_entries(range, &res)?;

        Ok(res)
    }

    /// Try to get an log entry.
    ///
    /// It does not return an error if the log entry at `log_index` is not found.
    async fn try_get_log_entry(&mut self, log_index: u64) -> Result<Option<Entry<C>>, StorageError<C>> {
        let mut res = self.try_get_log_entries(log_index..(log_index + 1)).await?;
        Ok(res.pop())
    }

    /// Returns the last deleted log id and the last log id.
    ///
    /// The impl should not consider the applied log id in state machine.
    /// The returned `last_log_id` could be the log id of the last present log entry, or the `last_purged_log_id` if
    /// there is no entry at all.
    // NOTE: This can be made into sync, provided all state machines will use atomic read or the like.
    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<C>>;

    /// Get a series of log entries from storage.
    ///
    /// The start value is inclusive in the search and the stop value is non-inclusive: `[start, stop)`.
    ///
    /// Entry that is not found is allowed.
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<C>>, StorageError<C>>;
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
    async fn build_snapshot(&mut self) -> Result<Snapshot<C, SD>, StorageError<C>>;

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
    // TODO(xp): simplify storage API

    /// The storage engine's associated type used for exposing a snapshot for reading & writing.
    ///
    /// See the [storage chapter of the guide](https://datafuselabs.github.io/openraft/getting-started.html#implement-raftstorage)
    /// for details on where and how this is used.
    type SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Send + Sync + Unpin + 'static;

    /// Log reader type.
    type LogReader: RaftLogReader<C>;

    /// Snapshot builder type.
    type SnapshotBuilder: RaftSnapshotBuilder<C, Self::SnapshotData>;

    /// Returns the last membership config found in log or state machine.
    async fn get_membership(&mut self) -> Result<Option<EffectiveMembership<C>>, StorageError<C>> {
        let (_, sm_mem) = self.last_applied_state().await?;

        let sm_mem_index = match &sm_mem {
            None => 0,
            Some(mem) => mem.log_id.index,
        };

        let log_mem = self.last_membership_in_log(sm_mem_index + 1).await?;

        if log_mem.is_some() {
            return Ok(log_mem);
        }

        return Ok(sm_mem);
    }

    /// Get the latest membership config found in the log.
    ///
    /// This method should returns membership with the greatest log index which is `>=since_index`.
    /// If no such membership log is found, it returns `None`, e.g., when logs are cleaned after being applied.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn last_membership_in_log(
        &mut self,
        since_index: u64,
    ) -> Result<Option<EffectiveMembership<C>>, StorageError<C>> {
        let st = self.get_log_state().await?;

        let mut end = st.last_log_id.next_index();
        let start = std::cmp::max(st.last_purged_log_id.next_index(), since_index);
        let step = 64;

        while start < end {
            let entries = self.try_get_log_entries(start..end).await?;

            for ent in entries.iter().rev() {
                if let EntryPayload::Membership(ref mem) = ent.payload {
                    return Ok(Some(EffectiveMembership::new(ent.log_id, mem.clone())));
                }
            }

            end = end.saturating_sub(step);
        }

        Ok(None)
    }

    /// Get Raft's state information from storage.
    ///
    /// When the Raft node is first started, it will call this interface to fetch the last known state from stable
    /// storage.
    async fn get_initial_state(&mut self) -> Result<InitialState<C>, StorageError<C>> {
        let vote = self.read_vote().await?;
        let st = self.get_log_state().await?;
        let mut last_log_id = st.last_log_id;
        let (last_applied, _) = self.last_applied_state().await?;
        let membership = self.get_membership().await?;

        // Clean up dirty state: snapshot is installed but logs are not cleaned.
        if last_log_id < last_applied {
            self.purge_logs_upto(last_applied.unwrap()).await?;
            last_log_id = last_applied;
        }

        Ok(InitialState {
            last_log_id,
            last_applied,
            vote: vote.unwrap_or_default(),
            last_membership: membership,
        })
    }

    /// Get the log id of the entry at `index`.
    async fn get_log_id(&mut self, log_index: u64) -> Result<LogId<C>, StorageError<C>> {
        let st = self.get_log_state().await?;

        if Some(log_index) == st.last_purged_log_id.index() {
            return Ok(st.last_purged_log_id.unwrap());
        }

        let entries = self.get_log_entries(log_index..=log_index).await?;

        Ok(entries[0].log_id)
    }

    // --- Vote

    async fn save_vote(&mut self, vote: &Vote<C>) -> Result<(), StorageError<C>>;

    async fn read_vote(&mut self) -> Result<Option<Vote<C>>, StorageError<C>>;

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
    async fn append_to_log(&mut self, entries: &[&Entry<C>]) -> Result<(), StorageError<C>>;

    /// Delete conflict log entries since `log_id`, inclusive.
    async fn delete_conflict_logs_since(&mut self, log_id: LogId<C>) -> Result<(), StorageError<C>>;

    /// Delete applied log entries upto `log_id`, inclusive.
    async fn purge_logs_upto(&mut self, log_id: LogId<C>) -> Result<(), StorageError<C>>;

    // --- State Machine

    /// Returns the last applied log id which is recorded in state machine, and the last applied membership log id and
    /// membership config.
    // NOTE: This can be made into sync, provided all state machines will use atomic read or the like.
    async fn last_applied_state(
        &mut self,
    ) -> Result<(Option<LogId<C>>, Option<EffectiveMembership<C>>), StorageError<C>>;

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
    async fn apply_to_state_machine(&mut self, entries: &[&Entry<C>]) -> Result<Vec<C::R>, StorageError<C>>;

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
    async fn begin_receiving_snapshot(&mut self) -> Result<Box<Self::SnapshotData>, StorageError<C>>;

    /// Install a snapshot which has finished streaming from the cluster leader.
    ///
    /// All other snapshots should be deleted at this point.
    ///
    /// ### snapshot
    /// A snapshot created from an earlier call to `begin_receiving_snapshot` which provided the snapshot.
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<C>,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<StateMachineChanges<C>, StorageError<C>>;

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
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<C, Self::SnapshotData>>, StorageError<C>>;
}

/// APIs for debugging a store.
#[async_trait]
pub trait RaftStorageDebug<SM> {
    /// Get a handle to the state machine for testing purposes.
    async fn get_state_machine(&mut self) -> SM;
}
