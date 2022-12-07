use futures::future::AbortHandle;

use crate::core::streaming_state::StreamingState;
use crate::Node;
use crate::NodeId;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::StorageError;

/// The current snapshot state of the Raft node.
pub(crate) enum SnapshotState<C: RaftTypeConfig, SD> {
    None,

    /// The Raft node is compacting itself.
    Snapshotting {
        /// A handle to abort the compaction process early if needed.
        abort_handle: AbortHandle,
    },
    /// The Raft node is streaming in a snapshot from the leader.
    Streaming(StreamingState<C, SD>),
}

impl<C: RaftTypeConfig, SD> Default for SnapshotState<C, SD> {
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
