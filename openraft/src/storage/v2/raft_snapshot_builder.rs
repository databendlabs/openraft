use openraft_macros::add_async_trait;

use crate::storage::Snapshot;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::StorageError;
/// A trait defining the interface for a Raft state machine snapshot subsystem.
///
/// This interface is accessed read-only from snapshot building task.
///
/// Typically, the snapshot implementation as such will be hidden behind a reference type like
/// `Arc<T>` or `Box<T>` and this interface implemented on the reference type. It can be
/// co-implemented with [`RaftStateMachine`] interface on the same cloneable object, if the
/// underlying state machine is anyway synchronized.
///
/// [`RaftStateMachine`]: crate::storage::RaftStateMachine
#[add_async_trait]
pub trait RaftSnapshotBuilder<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Build snapshot
    ///
    /// A snapshot has to contain state of all applied log, including membership. Usually it is just
    /// a serialized state machine.
    ///
    /// Building snapshot can be done by:
    /// - Performing log compaction, e.g. merge log entries that operates on the same key, like a
    ///   LSM-tree does,
    /// - or by fetching a snapshot from the state machine.
    async fn build_snapshot(&mut self) -> Result<Snapshot<C>, StorageError<C>>;

    // NOTES:
    // This interface is geared toward small file-based snapshots. However, not all snapshots can
    // be easily represented as a file. Probably a more generic interface will be needed to address
    // also other needs.
}
