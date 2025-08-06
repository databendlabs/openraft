use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::storage::Snapshot;
use crate::storage::SnapshotMeta;
use crate::type_config::alias::LogIdOf;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StoredMembership;

/// API for state machine and snapshot.
///
/// Snapshot is part of the state machine, because usually a snapshot is the persisted state of the
/// state machine.
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
    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembership<C>), StorageError<C>>;

    /// Apply the given payload of entries to the state machine.
    ///
    /// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
    /// have been replicated to a quorum of the cluster, will be applied to the state machine.
    ///
    /// This is where the business logic of interacting with your application's state machine
    /// should live. This is 100% application-specific. Perhaps this is where an application-specific
    /// transaction is being started, or perhaps committed. This may be where a key/value
    /// is being stored.
    ///
    /// For every entry to apply, an implementation should:
    /// - Store the log id as last-applied log id.
    /// - Deal with the business logic log.
    /// - Store membership config if `RaftEntry::get_membership()` returns `Some`.
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
    async fn apply<I>(&mut self, entries: I) -> Result<Vec<C::R>, StorageError<C>>
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
    /// See the [storage chapter of the guide][sto] for details on log compaction / snapshotting.
    ///
    /// [sto]: crate::docs::getting_started#3-implement-raftlogstorage-and-raftstatemachine
    #[since(version = "0.10.0", change = "SnapshotData without Box")]
    async fn begin_receiving_snapshot(&mut self) -> Result<C::SnapshotData, StorageError<C>>;

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
    #[since(version = "0.10.0", change = "SnapshotData without Box")]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<C>,
        snapshot: C::SnapshotData,
    ) -> Result<(), StorageError<C>>;

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
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<C>>, StorageError<C>>;
}
