//! Defines [`RaftLogStorage`] and [`RaftStateMachine`] trait to replace the previous
//! [`RaftStorage`](`crate::storage::RaftStorage`). [`RaftLogStorage`] is responsible for storing
//! logs, and [`RaftStateMachine`] is responsible for storing state machine and snapshot.

use macros::add_async_trait;

use crate::storage::callback::LogFlushed;
use crate::storage::v2::sealed::Sealed;
use crate::LogId;
use crate::LogState;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftLogReader;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::SnapshotMeta;
use crate::StorageError;
use crate::StoredMembership;
use crate::Vote;

pub(crate) mod sealed {
    /// Seal [`RaftLogStorage`](`crate::storage::RaftLogStorage`) and
    /// [`RaftStateMachine`](`crate::storage::RaftStateMachine`). This is to prevent users from
    /// implementing them before being stable.
    pub trait Sealed {}

    /// Implement non-public trait [`Sealed`] for all types so that [`RaftLogStorage`] and
    /// [`RaftStateMachine`] can be implemented by 3rd party crates.
    #[cfg(feature = "storage-v2")]
    impl<T> Sealed for T {}
}

/// API for log store.
///
/// `vote` API are also included because in raft, vote is part to the log: `vote` is about **when**,
/// while `log` is about **what**. A distributed consensus is about **at what a time, happened what
/// a event**.
///
/// ### To ensure correctness:
///
/// - Logs must be consecutive, i.e., there must **NOT** leave a **hole** in logs.
/// - All write-IO must be serialized, i.e., the internal implementation must **NOT** apply a latter
///   write request before a former write request is completed. This rule applies to both `vote` and
///   `log` IO. E.g., Saving a vote and appending a log entry must be serialized too.
#[add_async_trait]
pub trait RaftLogStorage<C>: Sealed + RaftLogReader<C> + OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Log reader type.
    ///
    /// Log reader is used by multiple replication tasks, which read logs and send them to remote
    /// nodes.
    type LogReader: RaftLogReader<C>;

    /// Returns the last deleted log id and the last log id.
    ///
    /// The impl should **not** consider the applied log id in state machine.
    /// The returned `last_log_id` could be the log id of the last present log entry, or the
    /// `last_purged_log_id` if there is no entry at all.
    // NOTE: This can be made into sync, provided all state machines will use atomic read or the
    // like.
    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<C::NodeId>>;

    /// Get the log reader.
    ///
    /// The method is intentionally async to give the implementation a chance to use asynchronous
    /// primitives to serialize access to the common internal object, if needed.
    async fn get_log_reader(&mut self) -> Self::LogReader;

    /// Save vote to storage.
    ///
    /// ### To ensure correctness:
    ///
    /// The vote must be persisted on disk before returning.
    async fn save_vote(&mut self, vote: &Vote<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;

    /// Return the last saved vote by [`Self::save_vote`].
    async fn read_vote(&mut self) -> Result<Option<Vote<C::NodeId>>, StorageError<C::NodeId>>;

    /// Saves the last committed log id to storage.
    ///
    /// # Optional feature
    ///
    /// If the state machine flushes state to disk before
    /// returning from `apply()`, then the application does not need to implement this method.
    /// Otherwise, this method is also optional(but not recommended), but your application has to
    /// deal with state reversion of state machine carefully upon restart. E.g., do not serve
    /// read operation a new `commit` message is received.
    ///
    /// See: [`docs::data::log_pointers`].
    ///
    /// [`docs::data::log_pointers`]: `crate::docs::data::log_pointers#optionally-persisted-committed`
    async fn save_committed(&mut self, _committed: Option<LogId<C::NodeId>>) -> Result<(), StorageError<C::NodeId>> {
        // By default `committed` log id is not saved
        Ok(())
    }

    /// Return the last saved committed log id by [`Self::save_committed`].
    async fn read_committed(&mut self) -> Result<Option<LogId<C::NodeId>>, StorageError<C::NodeId>> {
        // By default `committed` log id is not saved and this method just return None.
        Ok(None)
    }

    /// Append log entries and call the `callback` once logs are persisted on disk.
    ///
    /// It should returns immediately after saving the input log entries in memory, and calls the
    /// `callback` when the entries are persisted on disk, i.e., avoid blocking.
    ///
    /// This method is still async because preparing the IO is usually async.
    ///
    /// ### To ensure correctness:
    ///
    /// - When this method returns, the entries must be readable, i.e., a `LogReader` can read these
    ///   entries.
    ///
    /// - When the `callback` is called, the entries must be persisted on disk.
    ///
    ///   NOTE that: the `callback` can be called either before or after this method returns.
    ///
    /// - There must not be a **hole** in logs. Because Raft only examine the last log id to ensure
    ///   correctness.
    async fn append<I>(&mut self, entries: I, callback: LogFlushed<C::NodeId>) -> Result<(), StorageError<C::NodeId>>
    where
        I: IntoIterator<Item = C::Entry> + OptionalSend,
        I::IntoIter: OptionalSend;

    /// Truncate logs since `log_id`, inclusive
    ///
    /// ### To ensure correctness:
    ///
    /// - It must not leave a **hole** in logs.
    async fn truncate(&mut self, log_id: LogId<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;

    /// Purge logs upto `log_id`, inclusive
    ///
    /// ### To ensure correctness:
    ///
    /// - It must not leave a **hole** in logs.
    async fn purge(&mut self, log_id: LogId<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;
}

/// API for state machine and snapshot.
///
/// Snapshot is part of the state machine, because usually a snapshot is the persisted state of the
/// state machine.
#[add_async_trait]
pub trait RaftStateMachine<C>: Sealed + OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Snapshot builder type.
    type SnapshotBuilder: RaftSnapshotBuilder<C>;

    // TODO: This can be made into sync, provided all state machines will use atomic read or the
    //       like.
    // ---
    /// Returns the last applied log id which is recorded in state machine, and the last applied
    /// membership config.
    ///
    /// ### Correctness requirements
    ///
    /// It is all right to return a membership with greater log id than the
    /// last-applied-log-id.
    async fn applied_state(
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
    /// - Deal with the business logic log.
    /// - Store membership config if `RaftEntry::get_membership()` returns `Some`.
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
    async fn apply<I>(&mut self, entries: I) -> Result<Vec<C::R>, StorageError<C::NodeId>>
    where
        I: IntoIterator<Item = C::Entry> + OptionalSend,
        I::IntoIter: OptionalSend;

    /// Get the snapshot builder for the state machine.
    ///
    /// Usually it returns a snapshot view of the state machine(i.e., subsequent changes to the
    /// state machine won't affect the return snapshot view), or just a copy of the entire state
    /// machine.
    ///
    /// The method is intentionally async to give the implementation a chance to use
    /// asynchronous sync primitives to serialize access to the common internal object, if
    /// needed.
    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder;

    /// Create a new blank snapshot, returning a writable handle to the snapshot object.
    ///
    /// Openraft will use this handle to receive snapshot data.
    ///
    /// ### implementation guide
    ///
    /// See the [storage chapter of the guide](https://datafuselabs.github.io/openraft/storage.html)
    /// for details on snapshot streaming.
    async fn begin_receiving_snapshot(&mut self) -> Result<Box<C::SnapshotData>, StorageError<C::NodeId>>;

    /// Install a snapshot which has finished streaming from the leader.
    ///
    /// Before this method returns:
    /// - The state machine should be replaced with the new contents of the snapshot,
    /// - the input snapshot should be saved, i.e., [`Self::get_current_snapshot`] should return it.
    /// - and all other snapshots should be deleted at this point.
    ///
    /// ### snapshot
    ///
    /// A snapshot created from an earlier call to `begin_receiving_snapshot` which provided the
    /// snapshot.
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<C::NodeId, C::Node>,
        snapshot: Box<C::SnapshotData>,
    ) -> Result<(), StorageError<C::NodeId>>;

    /// Get a readable handle to the current snapshot.
    ///
    /// ### implementation algorithm
    ///
    /// Implementing this method should be straightforward. Check the configured snapshot
    /// directory for any snapshot files. A proper implementation will only ever have one
    /// active snapshot, though another may exist while it is being created. As such, it is
    /// recommended to use a file naming pattern which will allow for easily distinguishing between
    /// the current live snapshot, and any new snapshot which is being created.
    ///
    /// A proper snapshot implementation will store last-applied-log-id and the
    /// last-applied-membership config as part of the snapshot, which should be decoded for
    /// creating this method's response data.
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<C>>, StorageError<C::NodeId>>;
}
