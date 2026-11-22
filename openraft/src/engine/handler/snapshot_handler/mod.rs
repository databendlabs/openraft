//! Snapshot build admission and the snapshot metadata of the engine state.

use display_more::DisplayOptionExt;

use crate::RaftState;
use crate::RaftTypeConfig;
use crate::core::sm;
use crate::engine::Command;
use crate::engine::EngineOutput;
use crate::raft_state::LogStateReader;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::SnapshotMetaOf;

#[cfg(test)]
mod trigger_snapshot_test;
#[cfg(test)]
mod update_snapshot_test;

/// Starts snapshot builds, and records the metadata of the latest snapshot.
pub(crate) struct SnapshotHandler<'st, 'out, C, SM = ()>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) state: &'st mut RaftState<C>,
    pub(crate) output: &'out mut EngineOutput<C, SM>,
}

impl<'st, 'out, C, SM> SnapshotHandler<'st, 'out, C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(state: &'st mut RaftState<C>, output: &'out mut EngineOutput<C, SM>) -> Self {
        Self { state, output }
    }

    /// Queue a snapshot build, unless one is outstanding.
    ///
    /// Returns `true` if it queued a build, and `false` while an earlier build has not reported
    /// back.
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

    /// Replace the recorded snapshot metadata with `meta`, if `meta` has a greater last log id.
    ///
    /// Returns `true` if it replaced the metadata. With an equal or smaller last log id it leaves
    /// the metadata unchanged and returns `false`.
    ///
    /// The engine records only the metadata; the [`RaftStateMachine`] stores the snapshot data. A
    /// snapshot this node builds is recorded when the build completes. A snapshot from the Leader
    /// is recorded before its installation is queued, so the metadata can describe a snapshot that
    /// is not installed yet: the installation is done once the flushed snapshot progress reaches
    /// its last log id.
    ///
    /// [`RaftStateMachine`]: crate::storage::RaftStateMachine
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_snapshot(&mut self, meta: SnapshotMetaOf<C>) -> bool {
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
