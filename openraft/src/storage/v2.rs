//! Defines [`RaftLogStorage`] and [`RaftStateMachine`] trait to replace the previous
//! [`RaftStorage`](`crate::storage::RaftStorage`). [`RaftLogStorage`] is responsible for storing
//! logs, and [`RaftStateMachine`] is responsible for storing state machine and snapshot.

use async_trait::async_trait;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;

use crate::storage::callback::LogFlushed;
use crate::storage::v2::sealed::Sealed;
use crate::LogId;
use crate::RaftLogReader;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::SnapshotMeta;
use crate::StorageError;
use crate::StoredMembership;
use crate::Vote;

pub(crate) mod sealed {
    /// Seal [`RaftLogStorage`] and [`RaftStateMachine`]. This is to prevent users from implementing
    /// them before being stable.
    pub trait Sealed {}
}

#[async_trait]
pub trait RaftLogStorage<C>: Sealed + RaftLogReader<C> + Send + Sync + 'static
where C: RaftTypeConfig
{
    type LogReader: RaftLogReader<C>;

    async fn save_vote(&mut self, vote: &Vote<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;

    async fn read_vote(&mut self) -> Result<Option<Vote<C::NodeId>>, StorageError<C::NodeId>>;

    async fn get_log_reader(&mut self) -> Self::LogReader;

    /// Append log entries and call the `callback` once logs are persisted on disk.
    ///
    /// It should returns immediately after saving the input log entries in memory, and calls the
    /// `callback` when the entries are persisted on disk, i.e., avoid blocking.
    ///
    /// This method is still async because preparing preparing the IO is usually async.
    ///
    /// To ensure correctness:
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
    where I: IntoIterator<Item = C::Entry> + Send;

    /// Truncate logs since `log_id`, inclusive
    async fn truncate(&mut self, log_id: LogId<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;

    /// Purge logs upto `log_id`, inclusive
    async fn purge(&mut self, log_id: LogId<C::NodeId>) -> Result<(), StorageError<C::NodeId>>;
}

#[async_trait]
pub trait RaftStateMachine<C>: Sealed + Send + Sync + 'static
where C: RaftTypeConfig
{
    type SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Send + Sync + Unpin + 'static;

    type SnapshotBuilder: RaftSnapshotBuilder<C, Self::SnapshotData>;

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
    where I: IntoIterator<Item = C::Entry> + Send;

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder;

    async fn begin_receiving_snapshot(&mut self) -> Result<Box<Self::SnapshotData>, StorageError<C::NodeId>>;

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<C::NodeId, C::Node>,
        snapshot: Box<Self::SnapshotData>,
    ) -> Result<(), StorageError<C::NodeId>>;

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<C::NodeId, C::Node, Self::SnapshotData>>, StorageError<C::NodeId>>;
}
