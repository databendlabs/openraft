use futures::future::AbortHandle;
use tokio::sync::broadcast;

use crate::LogId;
use crate::Node;
use crate::NodeId;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::StorageError;

/// The current snapshot state of the Raft node.
pub(crate) enum SnapshotState<C: RaftTypeConfig> {
    None,

    /// The Raft node is compacting itself.
    Snapshotting {
        /// A handle to abort the compaction process early if needed.
        abort_handle: AbortHandle,
        /// A sender for notifying any other tasks of the completion of this compaction.
        sender: broadcast::Sender<Option<LogId<C::NodeId>>>,
    },
}

impl<C: RaftTypeConfig> Default for SnapshotState<C> {
    fn default() -> Self {
        SnapshotState::None
    }
}

/// Result of building a snapshot.
#[derive(Debug, Clone)]
pub(crate) enum SnapshotResult<NID: NodeId, N: Node> {
    /// Building snapshot has finished successfully.
    Ok(SnapshotMeta<NID, N>),

    /// Building snapshot encountered StorageError.
    StorageError(StorageError<NID>),

    /// Building snapshot is aborted by RaftCore.
    Aborted,
}
