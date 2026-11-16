use std::io;

use futures_util::Stream;
use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::storage::EntryResponder;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::SnapshotMetaOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::StoredMembershipOf;

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
    /// An opaque handle to snapshot contents, owned by the application.
    ///
    /// The generic core does not prepare directories, open files, flush writers, or rewind readers.
    /// The transport prepares a private receive target and completes and validates its transfer
    /// before passing the handle to
    /// [`Raft::install_full_snapshot`](crate::Raft::install_full_snapshot). A file transport
    /// must create missing parent directories, open for reading and writing, finish buffered
    /// writes, and position the handle where installation expects to read.
    /// [`Self::install_snapshot`] then validates the snapshot format and metadata and owns durable
    /// publication. Keep backing files alive for every reader, builder, transfer, and database
    /// handle that still uses them. Peer snapshot IDs are opaque protocol values, not local paths
    /// or evidence that a directory is safe to remove.
    ///
    /// See the [storage chapter of the guide][sto] for details on log compaction / snapshotting.
    ///
    /// [sto]: crate::docs::getting_started#3-implement-raftlogstorage-and-raftstatemachine
    #[since(version = "0.10.0", change = "moved from RaftTypeConfig to RaftStateMachine")]
    type SnapshotData: OptionalSend + 'static;

    /// Snapshot builder type.
    #[since(version = "0.10.0", change = "uses the state machine SnapshotData")]
    type SnapshotBuilder: RaftSnapshotBuilder<C, SnapshotData = Self::SnapshotData>;

    /// Returns the last applied log id which is recorded in state machine, and the last applied
    /// membership config.
    ///
    /// ### Correctness requirements
    ///
    /// It is all right to return a membership with greater log id than the
    /// last-applied-log-id.
    /// Because upon startup, the last membership will be loaded by scanning logs from the
    /// `last-applied-log-id`. Retain those logs until recovery no longer needs them.
    /// Application state must match the reported last-applied position after recovery, so replay
    /// neither skips missing effects nor repeats effects already represented by that position.
    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembershipOf<C>), io::Error>;

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
    /// For every entry to apply, an implementation must:
    /// - Store the log id as last-applied log id.
    /// - Deal with the business logic log.
    /// - Store membership config if `RaftEntry::get_membership()` returns `Some`.
    /// - Call [`ApplyResponder::send`](crate::storage::ApplyResponder::send) with the response
    ///   after applying the entry. If this is not called,
    ///   [`Raft::client_write`](crate::Raft::client_write) will never return.
    ///
    /// With relaxed reads
    /// ([`LinearizerOption::with_relaxed`](crate::raft::linearizable_read::LinearizerOption::with_relaxed)),
    /// state-machine reads after a successful write response must include that entry's updates.
    /// Sending a response does not wait for `apply()` to return. If a transaction is
    /// published only at the end of `apply()`, delay responses or the client-visible
    /// acknowledgement until publication.
    ///
    /// Note that for a membership log, the implementation needs to do nothing about it, except
    /// storing it.
    ///
    /// Openraft calls `apply()` at most once at a time. One call may cover several commit
    /// advances: while a call runs, the commits that arrive queue up, and the worker applies the
    /// queued ones together in a later call. Batches therefore grow with load: an implementation
    /// that sizes a transaction or a group commit by `apply()` call should expect one entry when
    /// writes arrive one at a time and hundreds when many clients write at once. With the
    /// `runtime-stats` feature, `RuntimeStats::apply_batch` reports the observed number of
    /// entries per call, and so does [`MetricsRecorder::record_apply_batch()`].
    ///
    /// The worker reads the next batch from the log, through
    /// [`RaftLogReader::entries_stream()`], while this call runs.
    ///
    /// An implementation may persist either the state machine or its snapshots:
    ///
    /// - A persistent state machine must durably store each acknowledged prefix's application
    ///   effects and apply progress before sending responses. Snapshots may be rebuilt from that
    ///   durable state. Atomic visibility alone does not establish machine-failure durability.
    /// - A transient state machine must publish effects to readers before sending responses and
    ///   retain durable logs needed after its last persistent snapshot. On restart, Openraft
    ///   installs that snapshot and replays logs through the persisted committed position.
    ///   Implement [`RaftLogStorage::save_committed()`] for this automatic replay.
    ///
    /// An `apply()` stream may commit successive prefixes. A later error may leave an earlier
    /// prefix durable; the entire call need not roll back. `Ok(())` must account for the whole
    /// supplied stream. Durable state, snapshots, and retained committed logs must reconstruct
    /// every acknowledged entry. The membership-ahead allowance in [`Self::applied_state`] still
    /// applies when the required logs remain available.
    ///
    /// [`RaftLogStorage::save_committed()`]: crate::storage::RaftLogStorage::save_committed
    /// [`MetricsRecorder::record_apply_batch()`]: crate::metrics::MetricsRecorder::record_apply_batch
    /// [`RaftLogReader::entries_stream()`]: crate::storage::RaftLogReader::entries_stream
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

    /// Install a snapshot which has finished streaming from the leader.
    ///
    /// This method is called in two scenarios:
    /// 1. **During replication**: When receiving a snapshot from the leader
    /// 2. **On restart**: Automatically called by [`StorageHelper::get_initial_state()`] to restore
    ///    transient state machines from the last persistent snapshot before creating [`Raft`]
    ///
    /// Validate the candidate's contents and metadata before replacing the live state. Prepare
    /// the replacement, durably select it, then publish its live handles. Before returning success,
    /// application state, [`Self::applied_state`], and [`Self::get_current_snapshot`] must describe
    /// the installed snapshot consistently.
    ///
    /// A persistent state machine must durably publish the replacement state; its snapshot files
    /// may be rebuilt after restart. A transient state machine must durably publish the replacement
    /// snapshot before covered logs can be purged. Retain the previous recovery state until its
    /// replacement is durable. If publication has an uncertain outcome, preserve every possibly
    /// selected generation and return the error so startup can resolve selection safely.
    /// Retired snapshots can be reclaimed after their last reader, builder, outgoing transfer,
    /// or database handle releases them.
    ///
    /// ### snapshot
    ///
    /// A complete snapshot to install: either received from the leader or returned by
    /// [`Self::get_current_snapshot`] when restoring on startup.
    ///
    /// [`StorageHelper::get_initial_state()`]: crate::StorageHelper::get_initial_state
    /// [`Raft`]: crate::Raft
    #[since(version = "0.10.0", change = "SnapshotData without Box")]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMetaOf<C>,
        snapshot: Self::SnapshotData,
    ) -> Result<(), io::Error>;

    /// Get a readable handle to the current snapshot.
    ///
    /// After restart, a persistent state machine may return `None` until it builds a snapshot.
    /// [`StorageHelper::get_initial_state`](crate::StorageHelper::get_initial_state) requests a
    /// new snapshot when purged logs require one. A transient state machine must expose its
    /// persistent recovery snapshot so startup can restore the state before replaying logs.
    ///
    /// Select the current snapshot explicitly; several retired snapshots may still have live
    /// owners. For file-backed recovery, use a durable selection record such as `CURRENT`,
    /// rather than inferring the selected generation from directory names or timestamps.
    /// Reclaim startup orphans only after opening and validating the selected generation under
    /// exclusive ownership of the storage directory.
    ///
    /// A proper snapshot implementation will store last-applied-log-id and the
    /// last-applied-membership config as part of the snapshot, which should be decoded for
    /// creating this method's response data.
    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<C, Self::SnapshotData>>, io::Error>;
}

// Test-only: an unconditional impl would downgrade a missing-state-machine mistake from a compile
// error to a runtime `Fatal`.
#[cfg(any(test, feature = "bench"))]
fn unit_state_machine_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "unit type does not implement a usable state machine",
    )
}

#[cfg(any(test, feature = "bench"))]
impl<C> RaftSnapshotBuilder<C> for ()
where C: RaftTypeConfig
{
    type SnapshotData = ();

    async fn build_snapshot(&mut self) -> Result<SnapshotOf<C, ()>, io::Error> {
        Err(unit_state_machine_error())
    }
}

#[cfg(any(test, feature = "bench"))]
impl<C> RaftStateMachine<C> for ()
where C: RaftTypeConfig
{
    type SnapshotData = ();

    type SnapshotBuilder = ();

    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembershipOf<C>), io::Error> {
        Err(unit_state_machine_error())
    }

    async fn apply<Strm>(&mut self, _entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<C>, io::Error>> + Unpin + OptionalSend {
        Err(unit_state_machine_error())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {}

    async fn install_snapshot(
        &mut self,
        _meta: &SnapshotMetaOf<C>,
        _snapshot: Self::SnapshotData,
    ) -> Result<(), io::Error> {
        Err(unit_state_machine_error())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<C, Self::SnapshotData>>, io::Error> {
        Err(unit_state_machine_error())
    }
}
