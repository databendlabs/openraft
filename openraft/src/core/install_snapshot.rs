use anyerror::AnyError;
use tokio::io::AsyncWriteExt;

use crate::core::streaming_state::StreamingState;
use crate::core::RaftCore;
use crate::core::SnapshotState;
use crate::error::InstallSnapshotError;
use crate::error::SnapshotMismatch;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::Entry;
use crate::ErrorSubject;
use crate::ErrorVerb;
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
    /// Leaders always send chunks in order. It is important to note that, according to the Raft spec,
    /// a log may only have one snapshot at any time. As snapshot contents are application specific,
    /// the Raft log will only store a pointer to the snapshot file along with the index & term.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) async fn handle_install_snapshot_request(
        &mut self,
        req: InstallSnapshotRequest<C>,
    ) -> Result<InstallSnapshotResponse<C::NodeId>, InstallSnapshotError<C::NodeId>> {
        tracing::debug!(req = display(req.summary()));

        let res = self.engine.handle_message_vote(&req.vote);
        self.run_engine_commands::<Entry<C>>(&[]).await?;
        if res.is_err() {
            tracing::info!(?self.engine.state.vote, %req.vote, "InstallSnapshot RPC term is less than current term, ignoring it.");
            return Ok(InstallSnapshotResponse {
                vote: self.engine.state.vote,
            });
        }

        self.set_next_election_time(false);

        // Clear the state to None if it is building a snapshot locally.
        if let SnapshotState::Snapshotting {
            abort_handle,
            join_handle,
        } = &mut self.snapshot_state
        {
            abort_handle.abort();

            // The building-snapshot task in another thread may still be running.
            // It has to block until it returns before dealing with snapshot streaming.
            // Otherwise there might be concurrency issue: installing the streaming snapshot and saving the built
            // snapshot may happen in any order.
            let _ = join_handle.await;
            self.snapshot_state = SnapshotState::None;
        }

        // Init a new streaming state if it is None.
        if let SnapshotState::None = self.snapshot_state {
            self.begin_installing_snapshot(&req).await?;
        }

        // It's Streaming.

        let done = req.done;
        let req_meta = req.meta.clone();

        // Changed to another stream. re-init snapshot state.
        let stream_changed = if let SnapshotState::Streaming(streaming) = &self.snapshot_state {
            req_meta.snapshot_id != streaming.snapshot_id
        } else {
            unreachable!("It has to be Streaming")
        };

        if stream_changed {
            self.begin_installing_snapshot(&req).await?;
        }

        // Receive the data.
        if let SnapshotState::Streaming(streaming) = &mut self.snapshot_state {
            debug_assert_eq!(req_meta.snapshot_id, streaming.snapshot_id);
            streaming.receive(req).await?;
        } else {
            unreachable!("It has to be Streaming")
        }

        if done {
            self.finalize_snapshot_installation(req_meta).await?;
        }

        Ok(InstallSnapshotResponse {
            vote: self.engine.state.vote,
        })
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn begin_installing_snapshot(
        &mut self,
        req: &InstallSnapshotRequest<C>,
    ) -> Result<(), InstallSnapshotError<C::NodeId>> {
        tracing::debug!(req = display(req.summary()));

        let id = req.meta.snapshot_id.clone();

        if req.offset > 0 {
            return Err(SnapshotMismatch {
                expect: SnapshotSegmentId {
                    id: id.clone(),
                    offset: 0,
                },
                got: SnapshotSegmentId { id, offset: req.offset },
            }
            .into());
        }

        let snapshot_data = self.storage.begin_receiving_snapshot().await?;
        self.snapshot_state = SnapshotState::Streaming(StreamingState::new(id, snapshot_data));

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

        let state = std::mem::take(&mut self.snapshot_state);
        let streaming = if let SnapshotState::Streaming(streaming) = state {
            streaming
        } else {
            unreachable!("snapshot_state has to be Streaming")
        };

        let mut snapshot_data = streaming.snapshot_data;

        snapshot_data.as_mut().shutdown().await.map_err(|e| StorageError::IO {
            source: StorageIOError::new(
                ErrorSubject::Snapshot(meta.signature()),
                ErrorVerb::Write,
                AnyError::new(&e),
            ),
        })?;

        // Buffer the snapshot data, let Engine decide to install it or to cancel it.
        self.received_snapshot.insert(meta.snapshot_id.clone(), snapshot_data);

        self.engine.install_snapshot(meta);
        self.run_engine_commands::<Entry<C>>(&[]).await?;

        Ok(())
    }
}
