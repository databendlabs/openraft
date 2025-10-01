use crate::RaftState;
use crate::RaftTypeConfig;
use crate::ServerState;
use crate::engine::EngineConfig;

#[cfg(test)]
mod update_server_state_test;

/// Handle raft server-state related operations
pub(crate) struct ServerStateHandler<'st, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'st EngineConfig<C>,
    pub(crate) state: &'st mut RaftState<C>,
}

impl<C> ServerStateHandler<'_, C>
where C: RaftTypeConfig
{
    /// Re-calculate the server-state if it changed, update the `server_state` field and dispatch
    /// commands to inform a runtime.
    pub(crate) fn update_server_state_if_changed(&mut self) {
        let server_state = self.state.calc_server_state(&self.config.id);

        tracing::debug!(
            id = display(&self.config.id),
            prev_server_state = debug(self.state.server_state),
            server_state = debug(server_state),
            "update_server_state_if_changed"
        );

        if self.state.server_state == server_state {
            return;
        }

        let was_leader = self.state.server_state == ServerState::Leader;
        let is_leader = server_state == ServerState::Leader;

        if !was_leader && is_leader {
            tracing::info!(id = display(&self.config.id), "become leader");
        } else if was_leader && !is_leader {
            tracing::info!(id = display(&self.config.id), "quit leader");
        } else {
            // nothing to do
        }

        self.state.server_state = server_state;
    }
}
