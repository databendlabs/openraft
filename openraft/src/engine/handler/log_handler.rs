use crate::engine::engine_impl::EngineOutput;
use crate::engine::Command;
use crate::raft_state::LogStateReader;
use crate::LogId;
use crate::Node;
use crate::NodeId;
use crate::RaftState;

/// Handle raft vote related operations
pub(crate) struct LogHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) state: &'x mut RaftState<NID, N>,
    pub(crate) output: &'x mut EngineOutput<NID, N>,
}

impl<'x, NID, N> LogHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Purge log entries upto `upto`, inclusive.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn purge_log(&mut self, upto: LogId<NID>) {
        tracing::info!(upto = display(&upto), "purge_log");
        // TODO: move tests into this mod
        let st = &mut self.state;
        let log_id = Some(&upto);

        if log_id <= st.last_purged_log_id() {
            return;
        }

        st.purge_log(&upto);

        self.output.push_command(Command::PurgeLog { upto });
    }
}
