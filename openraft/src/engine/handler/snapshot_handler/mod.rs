//! This mod handles state machine operations

use crate::RaftState;
use crate::RaftTypeConfig;
use crate::core::sm;
use crate::display_ext::DisplayOptionExt;
use crate::engine::Command;
use crate::engine::EngineOutput;
use crate::raft_state::LogStateReader;
use crate::storage::SnapshotMeta;

#[cfg(test)]
mod trigger_snapshot_test;
#[cfg(test)]
mod update_snapshot_test;

/// Handle raft vote related operations
pub(crate) struct SnapshotHandler<'st, 'out, C>
where C: RaftTypeConfig
{
    pub(crate) state: &'st mut RaftState<C>,
    pub(crate) output: &'out mut EngineOutput<C>,
}

impl<C> SnapshotHandler<'_, '_, C>
where C: RaftTypeConfig
{
    /// Trigger building a snapshot if there is no pending building job.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn trigger_snapshot(&mut self) -> bool {
        tracing::debug!("{}", func_name!());

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
    /// Engine records only the metadata of a snapshot. Snapshot data is stored by
    /// [`RaftStateMachine`] implementation.
    ///
    /// [`RaftStateMachine`]: crate::storage::RaftStateMachine
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_snapshot(&mut self, meta: SnapshotMeta<C>) -> bool {
        tracing::info!("update_snapshot: {:?}", meta);

        if meta.last_log_id.as_ref() <= self.state.snapshot_last_log_id() {
            tracing::info!(
                "No need to install a smaller snapshot: current snapshot last_log_id({}), new snapshot last_log_id({})",
                self.state.snapshot_last_log_id().display(),
                meta.last_log_id.display()
            );
            return false;
        }

        self.state.snapshot_meta = meta;

        true
    }
}
