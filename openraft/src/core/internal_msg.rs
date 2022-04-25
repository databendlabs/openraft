use crate::core::snapshot_state::SnapshotUpdate;
use crate::raft::VoteResponse;
use crate::NodeId;

/// Message for communication between internal tasks.
///
/// Such as log compaction task, RaftCore task, replication tasks etc.
#[derive(Debug, Clone)]
pub(crate) enum InternalMessage<NID: NodeId> {
    SnapshotUpdate(SnapshotUpdate<NID>),
    VoteResponse { target: NID, vote_resp: VoteResponse<NID> },
}
