use std::io::SeekFrom;
use std::sync::Arc;

use anyerror::AnyError;
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

use crate::core::RaftCore;
use crate::core::ServerState;
use crate::core::SnapshotState;
use crate::error::InstallSnapshotError;
use crate::error::SnapshotMismatch;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::LogIdOptionExt;
use crate::MembershipState;
use crate::MessageSummary;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::SnapshotSegmentId;
use crate::StorageError;
use crate::StorageIOError;

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    /// Invoked by leader to send chunks of a snapshot to a follower (§7).
    ///
    /// Leaders always send chunks in order. It is important to note that, according to the Raft spec,
    /// a log may only have one snapshot at any time. As snapshot contents are application specific,
    /// the Raft log will only store a pointer to the snapshot file along with the index & term.
    #[tracing::instrument(level = "debug", skip(self, req), fields(req=%req.summary()))]
    pub(super) async fn handle_install_snapshot_request(
        &mut self,
        req: InstallSnapshotRequest<C>,
    ) -> Result<InstallSnapshotResponse<C::NodeId>, InstallSnapshotError<C::NodeId>> {
        if req.vote < self.engine.state.vote {
            tracing::debug!(?self.engine.state.vote, %req.vote, "InstallSnapshot RPC term is less than current term");

            return Ok(InstallSnapshotResponse {
                vote: self.engine.state.vote,
            });
        }

        self.update_election_timeout();
        self.update_last_heartbeat();

        if req.vote > self.engine.state.vote {
            self.engine.state.vote = req.vote;
            self.save_vote().await?;

            // If not follower, become follower.
            if !self.engine.state.server_state.is_follower() && !self.engine.state.server_state.is_learner() {
                self.set_target_state(ServerState::Follower); // State update will emit metrics.
            }

            self.engine.metrics_flags.set_data_changed();
        }

        // Compare current snapshot state with received RPC and handle as needed.
        // - Init a new state if it is empty or building a snapshot locally.
        // - Mismatched id with offset=0 indicates a new stream has been sent, the old one should be dropped and start
        //   to receive the new snapshot,
        // - Mismatched id with offset greater than 0 is an out of order message that should be rejected.
        match self.snapshot_state.take() {
            None => {
                return self.begin_installing_snapshot(req).await;
            }
            Some(SnapshotState::Snapshotting { handle, .. }) => {
                handle.abort(); // Abort the current compaction in favor of installation from leader.
                return self.begin_installing_snapshot(req).await;
            }
            Some(SnapshotState::Streaming { snapshot, id, offset }) => {
                if req.meta.snapshot_id == id {
                    return self.continue_installing_snapshot(req, offset, snapshot).await;
                }

                if req.offset == 0 {
                    return self.begin_installing_snapshot(req).await;
                }

                Err(SnapshotMismatch {
                    expect: SnapshotSegmentId { id: id.clone(), offset },
                    got: SnapshotSegmentId {
                        id: req.meta.snapshot_id.clone(),
                        offset: req.offset,
                    },
                }
                .into())
            }
        }
    }

    #[tracing::instrument(level = "debug", skip(self, req), fields(req=%req.summary()))]
    async fn begin_installing_snapshot(
        &mut self,
        req: InstallSnapshotRequest<C>,
    ) -> Result<InstallSnapshotResponse<C::NodeId>, InstallSnapshotError<C::NodeId>> {
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

        // Create a new snapshot and begin writing its contents.
        let mut snapshot = self.storage.begin_receiving_snapshot().await?;
        snapshot.as_mut().write_all(&req.data).await.map_err(|e| StorageError::IO {
            source: StorageIOError::new(
                ErrorSubject::Snapshot(req.meta.clone()),
                ErrorVerb::Write,
                AnyError::new(&e),
            ),
        })?;

        // If this was a small snapshot, and it is already done, then finish up.
        if req.done {
            self.finalize_snapshot_installation(req, snapshot).await?;
            return Ok(InstallSnapshotResponse {
                vote: self.engine.state.vote,
            });
        }

        // Else, retain snapshot components for later segments & respond.
        self.snapshot_state = Some(SnapshotState::Streaming {
            offset: req.data.len() as u64,
            id,
            snapshot,
        });
        Ok(InstallSnapshotResponse {
            vote: self.engine.state.vote,
        })
    }

    #[tracing::instrument(level = "debug", skip(self, req, snapshot), fields(req=%req.summary()))]
    async fn continue_installing_snapshot(
        &mut self,
        req: InstallSnapshotRequest<C>,
        mut offset: u64,
        mut snapshot: Box<S::SnapshotData>,
    ) -> Result<InstallSnapshotResponse<C::NodeId>, InstallSnapshotError<C::NodeId>> {
        let id = req.meta.snapshot_id.clone();

        // Always seek to the target offset if not an exact match.
        if req.offset != offset {
            if let Err(err) = snapshot.as_mut().seek(SeekFrom::Start(req.offset)).await {
                self.snapshot_state = Some(SnapshotState::Streaming { offset, id, snapshot });
                return Err(StorageError::from_io_error(
                    ErrorSubject::Snapshot(req.meta.clone()),
                    ErrorVerb::Seek,
                    err,
                )
                .into());
            }
            offset = req.offset;
        }

        // Write the next segment & update offset.
        if let Err(err) = snapshot.as_mut().write_all(&req.data).await {
            self.snapshot_state = Some(SnapshotState::Streaming { offset, id, snapshot });
            return Err(
                StorageError::from_io_error(ErrorSubject::Snapshot(req.meta.clone()), ErrorVerb::Write, err).into(),
            );
        }
        offset += req.data.len() as u64;

        // If the snapshot stream is done, then finalize.
        if req.done {
            self.finalize_snapshot_installation(req, snapshot).await?;
        } else {
            self.snapshot_state = Some(SnapshotState::Streaming { offset, id, snapshot });
        }
        Ok(InstallSnapshotResponse {
            vote: self.engine.state.vote,
        })
    }

    /// Finalize the installation of a new snapshot.
    ///
    /// Any errors which come up from this routine will cause the Raft node to go into shutdown.
    #[tracing::instrument(level = "debug", skip(self, req, snapshot), fields(req=%req.summary()))]
    async fn finalize_snapshot_installation(
        &mut self,
        req: InstallSnapshotRequest<C>,
        mut snapshot: Box<S::SnapshotData>,
    ) -> Result<(), StorageError<C::NodeId>> {
        snapshot.as_mut().shutdown().await.map_err(|e| StorageError::IO {
            source: StorageIOError::new(
                ErrorSubject::Snapshot(req.meta.clone()),
                ErrorVerb::Write,
                AnyError::new(&e),
            ),
        })?;

        // Caveat: All changes to state machine must be serialized
        //
        // If `finalize_snapshot_installation` is run in RaftCore thread,
        // there is chance the last_applied being reset to a previous value:
        //
        // ```
        // RaftCore: -.    install-snapc,            .-> replicate_to_sm_handle.next(),
        //            |    update last_applied=5     |   update last_applied=2
        //            |                              |
        //            v                              |
        // task:      apply 2------------------------'
        // --------------------------------------------------------------------> time
        // ```

        // TODO(xp): do not install if self.engine.st.last_applied >= snapshot.meta.last_applied

        let snap_last_log_id = req.meta.last_log_id;

        // Unlike normal append-entries RPC, if conflicting logs are found, it is not **necessary** to delete them.
        // See: [Snapshot-replication](https://datafuselabs.github.io/openraft/replication.html#snapshot-replication)
        {
            let local = self.storage.try_get_log_entry(snap_last_log_id.index).await?;

            if let Some(local_log) = local {
                if local_log.log_id != snap_last_log_id {
                    tracing::info!(
                        local_log_id = display(&local_log.log_id),
                        snap_last_log_id = display(&snap_last_log_id),
                        "found conflict log id, when installing snapshot"
                    );
                }

                self.delete_conflict_logs_since(snap_last_log_id).await?;
            }
        }

        let st = &mut self.engine.state;

        let changes = self.storage.install_snapshot(&req.meta, snapshot).await?;
        tracing::debug!("update after apply or install-snapshot: {:?}", changes);

        let last_applied = changes.last_applied;

        if st.last_log_id < Some(last_applied) {
            st.last_log_id = Some(last_applied);
        }
        if st.committed < Some(last_applied) {
            st.committed = Some(last_applied);
        }
        if st.last_applied < Some(last_applied) {
            st.last_applied = Some(last_applied);
        }

        assert!(st.last_purged_log_id <= Some(last_applied));

        // A local log that is <= last_applied may be inconsistent with the leader.
        // It has to purge all of them to prevent these log form being replicated, when this node becomes leader.
        st.last_purged_log_id = Some(last_applied);
        self.storage.purge_logs_upto(last_applied).await?;

        {
            let snap_mem = req.meta.last_membership;
            let mut committed = st.membership_state.committed.clone();
            let mut effective = st.membership_state.effective.clone();
            if committed.log_id < snap_mem.log_id {
                committed = Arc::new(snap_mem.clone());
            }

            // The local effective membership may be inconsistent to the leader.
            // Thus it has to compare by log-index, e.g.:
            //   snap_mem.log_id        = (10, 5);
            //   local_effective.log_id = (2, 10);
            if effective.log_id.index() <= snap_mem.log_id.index() {
                effective = Arc::new(snap_mem);
            }

            let mem_state = MembershipState { committed, effective };
            tracing::debug!("update membership: {:?}", mem_state);

            self.update_membership(mem_state);
        }

        self.snapshot_last_log_id = self.engine.state.last_applied;
        self.engine.metrics_flags.set_data_changed();

        Ok(())
    }
}
