//! This mod handles state machine operations

use crate::core::sm;
use crate::engine::Command;
use crate::engine::EngineOutput;
use crate::raft_state::LogStateReader;
use crate::summary::MessageSummary;
use crate::AsyncRuntime;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;

#[cfg(test)] mod trigger_snapshot_test;
#[cfg(test)] mod update_snapshot_test;

/// Handle raft vote related operations
pub(crate) struct SnapshotHandler<'st, 'out, C>
where C: RaftTypeConfig
{
    pub(crate) state: &'st mut RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>,
    pub(crate) output: &'out mut EngineOutput<C>,
}

impl<'st, 'out, C> SnapshotHandler<'st, 'out, C>
where C: RaftTypeConfig
{
    /// Trigger building snapshot if there is no pending building job.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn trigger_snapshot(&mut self) -> bool {
        // TODO: test
        tracing::info!("{}", func_name!());

        if self.state.io_state_mut().building_snapshot() {
            tracing::debug!("snapshot building is in progress, do not trigger snapshot");
            return false;
        }

        tracing::info!("push snapshot building command");

        self.state.io_state.set_building_snapshot(true);

        self.output.push_command(Command::from(sm::Command::build_snapshot()));
        true
    }

    /// Update engine state when a new snapshot is built or installed.
    ///
    /// Engine records only the metadata of a snapshot. Snapshot data is stored by RaftStorage
    /// implementation.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_snapshot(&mut self, meta: SnapshotMeta<C::NodeId, C::Node>) -> bool {
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

        true
    }
}
