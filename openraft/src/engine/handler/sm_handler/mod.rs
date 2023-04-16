//! This mod handles state machine operations
use crate::engine::Command;
use crate::engine::EngineOutput;
use crate::entry::RaftEntry;
use crate::Node;
use crate::NodeId;
use crate::RaftState;

// TODO: add test
// #[cfg(test)] mod build_snapshot_test;

/// Handle raft vote related operations
pub(crate) struct StateMachineHandler<'st, 'out, NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N>,
{
    pub(crate) state: &'st mut RaftState<NID, N>,
    pub(crate) output: &'out mut EngineOutput<NID, N, Ent>,
}

impl<'st, 'out, NID, N, Ent> StateMachineHandler<'st, 'out, NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N>,
{
    /// Trigger building snapshot if there is no pending building job.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn build_snapshot(&mut self) -> bool {
        tracing::info!("{}", func_name!());

        if self.state.io_state_mut().building_snapshot() {
            tracing::debug!("snapshot building is in progress, do not trigger snapshot");
            return false;
        }

        tracing::info!("push snapshot building command");

        self.state.io_state.set_building_snapshot(true);
        self.output.push_command(Command::BuildSnapshot {});
        true
    }
}
