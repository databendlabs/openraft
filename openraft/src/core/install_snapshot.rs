use tokio::io::AsyncWriteExt;

use crate::core::streaming_state::Streaming;
use crate::core::RaftCore;
use crate::error::SnapshotMismatch;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::raft::InstallSnapshotTx;
use crate::MessageSummary;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::SnapshotSegmentId;
use crate::StorageError;
use crate::StorageIOError;

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    /// Invoked by leader to send chunks of a snapshot to a follower (§7).
    ///
    /// Leaders always send chunks in order. It is important to note that, according to the Raft
    /// spec, a log may only have one snapshot at any time. As snapshot contents are application
    /// specific, the Raft log will only store a pointer to the snapshot file along with the
    /// index & term.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) async fn handle_install_snapshot_request(
        &mut self,
        req: InstallSnapshotRequest<C>,
        tx: InstallSnapshotTx<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(req = display(req.summary()));

        let res = self.engine.vote_handler().handle_message_vote(&req.vote);
        self.run_engine_commands().await?;
        if res.is_err() {
            tracing::info!(
                my_vote = display(self.engine.state.vote_ref()),
                req_vote = display(&req.vote),
                "InstallSnapshot RPC term is less than current term, ignoring it."
            );
            let _ = tx.send(Ok(InstallSnapshotResponse {
                vote: *self.engine.state.vote_ref(),
            }));
            return Ok(());
        }

        let done = req.done;
        let req_meta = req.meta.clone();

        let curr_id = self.snapshot_state.streaming.as_ref().map(|s| &s.snapshot_id);

        // Changed to another stream. re-init snapshot state.
        if curr_id != Some(&req_meta.snapshot_id) {
            if let Err(e) = self.check_new_install_snapshot(&req) {
                let _ = tx.send(Err(e.into()));
                return Ok(());
            }
            self.begin_installing_snapshot(&req_meta).await?;
        }

        // Safe unwrap: it has been checked in the previous if statement.
        let streaming = self.snapshot_state.streaming.as_mut().unwrap();

        // Receive the data.
        streaming.receive(req).await?;

        if done {
            self.finalize_snapshot_installation(req_meta).await?;
            self.run_engine_commands().await?;
        }

        let _ = tx.send(Ok(InstallSnapshotResponse {
            vote: *self.engine.state.vote_ref(),
        }));

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    fn check_new_install_snapshot(&mut self, req: &InstallSnapshotRequest<C>) -> Result<(), SnapshotMismatch> {
        tracing::debug!(req = display(req.summary()));

        let id = req.meta.snapshot_id.clone();

        if req.offset > 0 {
            return Err(SnapshotMismatch {
                expect: SnapshotSegmentId {
                    id: id.clone(),
                    offset: 0,
                },
                got: SnapshotSegmentId { id, offset: req.offset },
            });
        }

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn begin_installing_snapshot(
        &mut self,
        meta: &SnapshotMeta<C::NodeId, C::Node>,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(req = display(meta.summary()));

        let snapshot_data = self.storage.begin_receiving_snapshot().await?;

        let id = meta.snapshot_id.clone();
        self.snapshot_state.streaming = Some(Streaming::new(id, snapshot_data));

        Ok(())
    }

    /// Finalize the installation of a new snapshot.
    ///
    /// Any errors which come up from this routine will cause the Raft node to go into shutdown.
    #[tracing::instrument(level = "debug", skip_all)]
    async fn finalize_snapshot_installation(
        &mut self,
        meta: SnapshotMeta<C::NodeId, C::Node>,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(meta = display(meta.summary()));

        let streaming = self.snapshot_state.streaming.take().unwrap();

        let mut snapshot_data = streaming.snapshot_data;

        snapshot_data
            .as_mut()
            .shutdown()
            .await
            .map_err(|e| StorageIOError::write_snapshot(meta.signature(), &e))?;

        // Buffer the snapshot data, let Engine decide to install it or to cancel it.
        self.received_snapshot.insert(meta.snapshot_id.clone(), snapshot_data);

        self.engine.following_handler().install_snapshot(meta);

        Ok(())
    }
}
