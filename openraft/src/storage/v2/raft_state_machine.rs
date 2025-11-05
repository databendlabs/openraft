use std::io;

use futures::Stream;
use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::StoredMembership;
use crate::storage::EntryResponder;
use crate::storage::Snapshot;
use crate::storage::SnapshotMeta;
use crate::type_config::alias::LogIdOf;

/// API for state machine and snapshot.
///
/// Snapshot is part of the state machine, because usually a snapshot is the persisted state of the
/// state machine.
///
/// ## Related Types
///
/// - [`RaftLogStorage`](crate::storage::RaftLogStorage) - For log storage operations
/// - [`RaftNetworkFactory`](crate::network::RaftNetworkFactory) - For network communication
/// - [`Config`](crate::config::Config) - For configuration options
///
/// See: [`StateMachine`](crate::docs::components::state_machine)
#[add_async_trait]
pub trait RaftStateMachine<C>: OptionalSend + OptionalSync + 'static
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
    /// Because upon startup, the last membership will be loaded by scanning logs from the
    /// `last-applied-log-id`.
    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembership<C>), io::Error>;

    /// Apply the given payload of entries to the state machine.
    ///
    /// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
    /// have been replicated to a quorum of the cluster, will be applied to the state machine.
    ///
    /// This is where the business logic of interacting with your application's state machine
    /// should live. This is 100% application-specific. Perhaps this is where an
    /// application-specific transaction is being started, or perhaps committed. This may be
    /// where a key/value is being stored.
    ///
    /// For every entry to apply, an implementation should:
    /// - Store the log id as last-applied log id.
    /// - Deal with the business logic log.
    /// - Store membership config if `RaftEntry::get_membership()` returns `Some`.
    /// - Call [`ApplyResponder::send`](crate::storage::ApplyResponder::send) with the response
    ///   immediately after applying the entry.
    ///
    /// Note that for a membership log, the implementation needs to do nothing about it, except
    /// storing it.
    ///
    /// An implementation may choose to persist either the state machine or the snapshot:
    ///
    /// - An implementation with a persistent state machine: persists the state on disk before
    ///   returning from `apply()`. So that a snapshot does not need to be persistent.
    ///
    /// - An implementation with a persistent snapshot: `apply()` does not have to persist state on
    ///   disk. But every snapshot has to be persistent. And when starting up the application, the
    ///   state machine should be rebuilt from the last snapshot.
    ///
    /// - An implementation with a transient (in-memory) state machine: The state machine does NOT
    ///   persist on each `apply()` call, but snapshots ARE persistent. On restart, in-memory state
    ///   is lost and `applied_state()` returns `None` or an old value. Openraft will automatically
    ///   restore the state machine by: (1) installing the last persistent snapshot, then (2)
    ///   re-applying logs from the snapshot position to the committed position. To support this
    ///   pattern, implement [`RaftLogStorage::save_committed()`] to persist the committed log id.
    ///
    /// [`RaftLogStorage::save_committed()`]: crate::storage::RaftLogStorage::save_committed
    #[since(version = "0.10.0", change = "Entry-Responder-Result stream")]
    async fn apply<Strm>(&mut self, entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<C>, io::Error>> + Unpin + OptionalSend;

    /// Try to create a snapshot builder for the state machine.
    ///
    /// Returns a snapshot view of the state machine, or `None` to defer snapshot creation.
    ///
    /// This allows applications to optimize snapshot timing based on system load, I/O conditions,
    /// or other operational constraints. For example, deferring snapshots during high write load
    /// or waiting for quieter periods.
    ///
    /// # Arguments
    ///
    /// - `force`: Controls whether snapshot creation can be deferred:
    ///   - `true`: Implementation **must** return `Some(builder)`. OpenRaft uses this when a
    ///     snapshot is required for replication (e.g., logs have been purged and a follower needs
    ///     to catch up via snapshot).
    ///   - `false`: Implementation **may** return `None` to defer snapshot creation. OpenRaft uses
    ///     this for policy-based snapshots triggered by `SnapshotPolicy`.
    ///
    /// # Default Implementation
    ///
    /// Delegates to [`Self::get_snapshot_builder`] for backward compatibility. New implementations
    /// should override this method instead of [`Self::get_snapshot_builder`].
    #[since(version = "0.10.0")]
    async fn try_create_snapshot_builder(&mut self, force: bool) -> Option<Self::SnapshotBuilder> {
        let _ = force;
        Some(self.get_snapshot_builder().await)
    }

    /// Get the snapshot builder for the state machine.
    ///
    /// Returns a snapshot view of the state machine (subsequent changes won't affect the view).
    ///
    /// This method will be replaced by [`Self::try_create_snapshot_builder`] in the future.
    #[since(version = "0.10.0", change = "deprecated, use `try_create_snapshot_builder` instead")]
    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder;

    /// Create a new blank snapshot, returning a writable handle to the snapshot object.
    ///
    /// Openraft will use this handle to receive snapshot data.
    ///
    /// See the [storage chapter of the guide][sto] for details on log compaction / snapshotting.
    ///
    /// [sto]: crate::docs::getting_started#3-implement-raftlogstorage-and-raftstatemachine
    #[since(version = "0.10.0", change = "SnapshotData without Box")]
    async fn begin_receiving_snapshot(&mut self) -> Result<C::SnapshotData, io::Error>;

    /// Install a snapshot which has finished streaming from the leader.
    ///
    /// This method is called in two scenarios:
    /// 1. **During replication**: When receiving a snapshot from the leader
    /// 2. **On restart**: Automatically called by [`StorageHelper::get_initial_state()`] to restore
    ///    transient state machines from the last persistent snapshot before creating [`Raft`]
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
    ///
    /// [`StorageHelper::get_initial_state()`]: crate::StorageHelper::get_initial_state
    /// [`Raft`]: crate::Raft
    #[since(version = "0.10.0", change = "SnapshotData without Box")]
    async fn install_snapshot(&mut self, meta: &SnapshotMeta<C>, snapshot: C::SnapshotData) -> Result<(), io::Error>;

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
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<C>>, io::Error>;
}
