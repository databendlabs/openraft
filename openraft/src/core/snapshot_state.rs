use crate::core::building_state::Building;
use crate::core::streaming_state::Streaming;
use crate::LeaderId;
use crate::Node;
use crate::NodeId;
use crate::SnapshotId;
use crate::SnapshotMeta;
use crate::StorageError;

/// A global unique id of install-snapshot request.
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub(crate) struct SnapshotRequestId<NID: NodeId> {
    pub(crate) leader_id: LeaderId<NID>,
    pub(crate) snapshot_id: SnapshotId,
    pub(crate) offset: u64,
}

impl<NID: NodeId> SnapshotRequestId<NID> {
    pub(crate) fn new(leader_id: LeaderId<NID>, snapshot_id: SnapshotId, offset: u64) -> Self {
        Self {
            leader_id,
            snapshot_id,
            offset,
        }
    }
}

/// The current snapshot state of the Raft node.
///
/// There can be a building process and a streaming process at the same time.
/// When receiving or building a snapshot is done, the snapshot with a greater last-log-id will be
/// installed.
pub(crate) struct State<SD> {
    /// The Raft node is streaming in a snapshot from the leader.
    pub(crate) streaming: Option<Streaming<SD>>,

    /// The Raft node is building snapshot itself.
    pub(crate) building: Option<Building>,
}

impl<SD> Default for State<SD> {
    fn default() -> Self {
        Self {
            streaming: None,
            building: None,
        }
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
