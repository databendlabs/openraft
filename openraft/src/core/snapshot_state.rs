use crate::LeaderId;
use crate::NodeId;
use crate::SnapshotId;

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
