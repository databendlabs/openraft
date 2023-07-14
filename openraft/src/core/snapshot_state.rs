use crate::LeaderId;
use crate::NodeId;
use crate::SnapshotId;

/// A global unique id of install-snapshot request.
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub(crate) struct SnapshotRequestId<NID: NodeId, C> {
    pub(crate) leader_id: LeaderId<NID>,
    pub(crate) snapshot_id: SnapshotId,
    pub(crate) chunk_id: Option<C>,
}

impl<NID: NodeId, C> SnapshotRequestId<NID, C> {
    pub(crate) fn new(leader_id: LeaderId<NID>, snapshot_id: SnapshotId, chunk_id: Option<C>) -> Self {
        Self {
            leader_id,
            snapshot_id,
            chunk_id,
        }
    }
}
