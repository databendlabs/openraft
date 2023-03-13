use crate::engine::engine_impl::EngineOutput;
use crate::raft_state::LogStateReader;
use crate::summary::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RaftState;
use crate::SnapshotMeta;

#[cfg(test)] mod update_snapshot_test;

/// Handle raft vote related operations
pub(crate) struct SnapshotHandler<'st, 'out, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) state: &'st mut RaftState<NID, N>,
    pub(crate) output: &'out mut EngineOutput<NID, N>,
}

impl<'st, 'out, NID, N> SnapshotHandler<'st, 'out, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Update engine state when a new snapshot is built or installed.
    ///
    /// Engine records only the metadata of a snapshot. Snapshot data is stored by RaftStorage
    /// implementation.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_snapshot(&mut self, meta: SnapshotMeta<NID, N>) -> bool {
        tracing::info!("update_snapshot: {:?}", meta);

        if meta.last_log_id <= self.state.snapshot_last_log_id().copied() {
            tracing::info!(
                "No need to install a smaller snapshot: current snapshot last_log_id({}), new snapshot last_log_id({})",
                self.state.snapshot_last_log_id().summary(),
                meta.last_log_id.summary()
            );
            return false;
        }

        self.state.snapshot_meta = meta;
        self.output.metrics_flags.set_data_changed();

        true
    }
}
