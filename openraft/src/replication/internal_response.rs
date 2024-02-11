//! Responses for ReplicationCore internal communication.
use core::fmt;

use crate::error::Fatal;
use crate::error::StreamingError;
use crate::raft::SnapshotResponse;
use crate::type_config::alias::InstantOf;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;

#[derive(Debug)]
pub(crate) struct ReplicateSnapshotResponse<C: RaftTypeConfig> {
    /// The time when the snapshot replication started on leader.
    ///
    /// This time is used to extend lease of the leader on leader.
    pub(crate) start_time: InstantOf<C>,

    /// Meta data of the snapshot to be replicated.
    pub(crate) snapshot_meta: SnapshotMeta<C::NodeId, C::Node>,

    /// The result of the snapshot replication.
    pub(crate) result: Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>>,
}

impl<C: RaftTypeConfig> ReplicateSnapshotResponse<C> {
    pub(in crate::replication) fn new(
        start_time: InstantOf<C>,
        snapshot_meta: SnapshotMeta<C::NodeId, C::Node>,
        result: Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>>,
    ) -> Self {
        Self {
            start_time,
            snapshot_meta,
            result,
        }
    }
}

impl<C: RaftTypeConfig> fmt::Display for ReplicateSnapshotResponse<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SnapshotResponse {{ start_time: {:?}, snapshot_meta: {}, result:",
            self.start_time, self.snapshot_meta
        )?;

        if let Ok(resp) = &self.result {
            write!(f, " Ok({})", resp)?;
        } else {
            write!(f, " Err({})", self.result.as_ref().unwrap_err())?;
        };

        write!(f, " }}",)
    }
}
