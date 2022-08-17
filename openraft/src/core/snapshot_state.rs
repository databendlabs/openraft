use futures::future::AbortHandle;
use tokio::sync::broadcast;

use crate::LogId;
use crate::NodeId;
use crate::RaftTypeConfig;

/// The current snapshot state of the Raft node.
pub(crate) enum SnapshotState<C: RaftTypeConfig, SD> {
    /// The Raft node is compacting itself.
    Snapshotting {
        /// A handle to abort the compaction process early if needed.
        handle: AbortHandle,
        /// A sender for notifying any other tasks of the completion of this compaction.
        sender: broadcast::Sender<Option<LogId<C::NodeId>>>,
    },
    /// The Raft node is streaming in a snapshot from the leader.
    Streaming {
        /// The offset of the last byte written to the snapshot.
        offset: u64,
        /// The ID of the snapshot being written.
        id: String,
        /// A handle to the snapshot writer.
        snapshot: Box<SD>,
    },
}

/// An update on a snapshot creation process.
#[derive(Debug, Clone)]
pub(crate) enum SnapshotUpdate<NID: NodeId> {
    /// Snapshot creation has finished successfully and covers the given index.
    SnapshotComplete(Option<LogId<NID>>),

    /// Snapshot creation failed.
    SnapshotFailed,
}
