use crate::SnapshotId;

/// The Raft node is streaming in a snapshot from the leader.
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub(crate) struct StreamingState {
    /// The offset of the last byte written to the snapshot.
    pub(crate) offset: u64,

    /// The ID of the snapshot being written.
    pub(crate) snapshot_id: SnapshotId,
}
