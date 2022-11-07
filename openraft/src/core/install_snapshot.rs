use crate::core::RaftCore;
use crate::core::SnapshotState;
use crate::error::InstallSnapshotError;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::Entry;
use crate::MessageSummary;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    /// Invoked by leader to send chunks of a snapshot to a follower (§7).
    ///
    /// Leaders always send chunks in order. It is important to note that, according to the Raft spec,
    /// a log may only have one snapshot at any time. As snapshot contents are application specific,
    /// the Raft log will only store a pointer to the snapshot file along with the index & term.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) async fn handle_install_snapshot_request(
        &mut self,
        req: InstallSnapshotRequest<C>,
    ) -> Result<InstallSnapshotResponse<C::NodeId>, InstallSnapshotError<C::NodeId>> {
        tracing::debug!(req = display(req.summary()));

        let res = self.engine.handle_vote_change(&req.vote);
        self.run_engine_commands::<Entry<C>>(&[]).await?;
        if res.is_err() {
            tracing::info!(?self.engine.state.vote, %req.vote, "InstallSnapshot RPC term is less than current term, ignoring it.");
            return Ok(InstallSnapshotResponse {
                vote: self.engine.state.vote,
            });
        }

        self.set_next_election_time(false);

        // Clear the state to None if it is building a snapshot locally.
        if let SnapshotState::Snapshotting { abort_handle, .. } = &mut self.snapshot_state {
            abort_handle.abort(); // Abort the current compaction in favor of installation from leader.
            self.snapshot_state = SnapshotState::None;
        }

        let req_meta = req.meta.clone();

        self.received_snapshot.insert(req_meta.snapshot_id.clone(), req.data);

        self.engine.install_snapshot(req_meta);
        self.run_engine_commands::<Entry<C>>(&[]).await?;

        Ok(InstallSnapshotResponse {
            vote: self.engine.state.vote,
        })
    }
}
