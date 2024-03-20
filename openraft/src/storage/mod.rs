//! The Raft storage interface and data types.

mod callback;
mod helper;
mod log_store_ext;
mod snapshot_signature;
mod v2;

use std::fmt;
use std::fmt::Debug;
use std::ops::RangeBounds;

pub use helper::StorageHelper;
pub use log_store_ext::RaftLogReaderExt;
use openraft_macros::add_async_trait;
pub use snapshot_signature::SnapshotSignature;
pub use v2::RaftLogStorage;
pub use v2::RaftLogStorageExt;
pub use v2::RaftStateMachine;

use crate::display_ext::DisplayOption;
use crate::raft_types::SnapshotId;
pub use crate::storage::callback::LogApplied;
pub use crate::storage::callback::LogFlushed;
use crate::LogId;
use crate::MessageSummary;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StoredMembership;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct SnapshotMeta<C>
where C: RaftTypeConfig
{
    /// Log entries upto which this snapshot includes, inclusive.
    pub last_log_id: Option<LogId<C::NodeId>>,

    /// The last applied membership config.
    pub last_membership: StoredMembership<C>,

    /// To identify a snapshot when transferring.
    /// Caveat: even when two snapshot is built with the same `last_log_id`, they still could be
    /// different in bytes.
    pub snapshot_id: SnapshotId,
}

impl<C> fmt::Display for SnapshotMeta<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{snapshot_id: {}, last_log:{}, last_membership: {}}}",
            self.snapshot_id,
            DisplayOption(&self.last_log_id),
            self.last_membership
        )
    }
}

impl<C> MessageSummary<SnapshotMeta<C>> for SnapshotMeta<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        self.to_string()
    }
}

impl<C> SnapshotMeta<C>
where C: RaftTypeConfig
{
    pub fn signature(&self) -> SnapshotSignature<C::NodeId> {
        SnapshotSignature {
            last_log_id: self.last_log_id,
            last_membership_log_id: *self.last_membership.log_id(),
            snapshot_id: self.snapshot_id.clone(),
        }
    }

    /// Returns a ref to the id of the last log that is included in this snapshot.
    pub fn last_log_id(&self) -> Option<&LogId<C::NodeId>> {
        self.last_log_id.as_ref()
    }
}

/// The data associated with the current snapshot.
#[derive(Debug, Clone)]
pub struct Snapshot<C>
where C: RaftTypeConfig
{
    /// metadata of a snapshot
    pub meta: SnapshotMeta<C>,

    /// A read handle to the associated snapshot.
    pub snapshot: Box<C::SnapshotData>,
}

impl<C> Snapshot<C>
where C: RaftTypeConfig
{
    #[allow(dead_code)]
    pub(crate) fn new(meta: SnapshotMeta<C>, snapshot: Box<C::SnapshotData>) -> Self {
        Self { meta, snapshot }
    }
}

impl<C> fmt::Display for Snapshot<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Snapshot{{meta: {}}}", self.meta)
    }
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
/// this interface implemented on the `Arc<T>`. It can be co-implemented with [`RaftLogStorage`]
/// interface on the same cloneable object, if the underlying state machine is anyway synchronized.
#[add_async_trait]
pub trait RaftLogReader<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Get a series of log entries from storage.
    ///
    /// The start value is inclusive in the search and the stop value is non-inclusive: `[start,
    /// stop)`.
    ///
    /// Entry that is not found is allowed.
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
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
/// co-implemented with [`RaftStateMachine`] interface on the same cloneable object, if the
/// underlying state machine is anyway synchronized.
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
    async fn build_snapshot(&mut self) -> Result<Snapshot<C>, StorageError<C::NodeId>>;

    // NOTES:
    // This interface is geared toward small file-based snapshots. However, not all snapshots can
    // be easily represented as a file. Probably a more generic interface will be needed to address
    // also other needs.
}
