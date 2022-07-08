use crate::core::RaftCore;
use crate::error::AppendEntriesError;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) async fn handle_append_entries_request(
        &mut self,
        req: AppendEntriesRequest<C>,
    ) -> Result<AppendEntriesResponse<C::NodeId>, AppendEntriesError<C::NodeId>> {
        let resp = self.engine.handle_append_entries_req(&req.vote, req.prev_log_id, &req.entries, req.leader_commit);
        self.run_engine_commands(req.entries.as_slice()).await?;

        Ok(resp)
    }

    /// Replicate any outstanding entries to the state machine for which it is safe to do so.
    ///
    /// Very importantly, this routine must not block the main control loop main task, else it
    /// may cause the Raft leader to timeout the requests to this node.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) async fn replicate_to_state_machine_if_needed(&mut self) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(?self.engine.state.last_applied, ?self.engine.state.committed, "replicate_to_sm_if_needed");

        // If we don't have any new entries to replicate, then do nothing.
        if self.engine.state.committed <= self.engine.state.last_applied {
            tracing::debug!(
                "committed({:?}) <= last_applied({:?}), return",
                self.engine.state.committed,
                self.engine.state.last_applied
            );
            // TODO(xp): this should be moved to upper level.
            self.engine.metrics_flags.set_data_changed();
            return Ok(());
        }

        if let Some(c) = self.engine.state.committed {
            self.apply_to_state_machine(c.index).await?;
        }

        Ok(())
    }
}
