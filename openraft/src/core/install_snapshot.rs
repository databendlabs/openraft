use std::io::SeekFrom;

use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

use crate::core::delete_applied_logs;
use crate::core::RaftCore;
use crate::core::SnapshotState;
use crate::core::State;
use crate::core::UpdateCurrentLeader;
use crate::error::RaftResult;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::AppData;
use crate::AppDataResponse;
use crate::MessageSummary;
use crate::RaftError;
use crate::RaftNetwork;
use crate::RaftStorage;
use crate::SnapshotSegmentId;
use crate::Update;

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> RaftCore<D, R, N, S> {
    /// Invoked by leader to send chunks of a snapshot to a follower (§7).
    ///
    /// Leaders always send chunks in order. It is important to note that, according to the Raft spec,
    /// a log may only have one snapshot at any time. As snapshot contents are application specific,
    /// the Raft log will only store a pointer to the snapshot file along with the index & term.
    #[tracing::instrument(level = "debug", skip(self, req), fields(req=%req.summary()))]
    pub(super) async fn handle_install_snapshot_request(
        &mut self,
        req: InstallSnapshotRequest,
    ) -> RaftResult<InstallSnapshotResponse> {
        // If message's term is less than most recent term, then we do not honor the request.
        if req.term < self.current_term {
            return Ok(InstallSnapshotResponse {
                term: self.current_term,
            });
        }

        // Update election timeout.
        self.update_next_election_timeout(true);

        // Update current term if needed.
        let mut report_metrics = false;
        if self.current_term != req.term {
            self.update_current_term(req.term, None);
            self.save_hard_state().await?;
            report_metrics = true;
        }

        // Update current leader if needed.
        if self.current_leader.as_ref() != Some(&req.leader_id) {
            self.update_current_leader(UpdateCurrentLeader::OtherNode(req.leader_id));
            report_metrics = true;
        }

        // If not follower, become follower.
        if !self.target_state.is_follower() && !self.target_state.is_learner() {
            self.set_target_state(State::Follower); // State update will emit metrics.
        }

        if report_metrics {
            self.report_metrics(Update::Ignore);
        }

        // Compare current snapshot state with received RPC and handle as needed.
        // - Init a new state if it is empty or building a snapshot locally.
        // - Mismatched id with offset=0 indicates a new stream has been sent, the old one should be dropped and start
        //   to receive the new snapshot,
        // - Mismatched id with offset greater than 0 is an out of order message that should be rejected.
        match self.snapshot_state.take() {
            None => self.begin_installing_snapshot(req).await,
            Some(SnapshotState::Snapshotting { handle, .. }) => {
                handle.abort(); // Abort the current compaction in favor of installation from leader.
                self.begin_installing_snapshot(req).await
            }
            Some(SnapshotState::Streaming { snapshot, id, offset }) => {
                if req.meta.snapshot_id == id {
                    return self.continue_installing_snapshot(req, offset, snapshot).await;
                }

                if req.offset == 0 {
                    return self.begin_installing_snapshot(req).await;
                }

                Err(RaftError::SnapshotMismatch {
                    expect: SnapshotSegmentId { id: id.clone(), offset },
                    got: SnapshotSegmentId {
                        id: req.meta.snapshot_id.clone(),
                        offset: req.offset,
                    },
                })
            }
        }
    }

    #[tracing::instrument(level = "debug", skip(self, req), fields(req=%req.summary()))]
    async fn begin_installing_snapshot(&mut self, req: InstallSnapshotRequest) -> RaftResult<InstallSnapshotResponse> {
        let id = req.meta.snapshot_id.clone();

        if req.offset > 0 {
            return Err(RaftError::SnapshotMismatch {
                expect: SnapshotSegmentId {
                    id: id.clone(),
                    offset: 0,
                },
                got: SnapshotSegmentId { id, offset: req.offset },
            });
        }

        // Create a new snapshot and begin writing its contents.
        let mut snapshot = self.storage.begin_receiving_snapshot().await.map_err(|err| self.map_storage_error(err))?;
        snapshot.as_mut().write_all(&req.data).await?;

        // If this was a small snapshot, and it is already done, then finish up.
        if req.done {
            self.finalize_snapshot_installation(req, snapshot).await?;
            return Ok(InstallSnapshotResponse {
                term: self.current_term,
            });
        }

        // Else, retain snapshot components for later segments & respond.
        self.snapshot_state = Some(SnapshotState::Streaming {
            offset: req.data.len() as u64,
            id,
            snapshot,
        });
        Ok(InstallSnapshotResponse {
            term: self.current_term,
        })
    }

    #[tracing::instrument(level = "debug", skip(self, req, snapshot), fields(req=%req.summary()))]
    async fn continue_installing_snapshot(
        &mut self,
        req: InstallSnapshotRequest,
        mut offset: u64,
        mut snapshot: Box<S::SnapshotData>,
    ) -> RaftResult<InstallSnapshotResponse> {
        let id = req.meta.snapshot_id.clone();

        // Always seek to the target offset if not an exact match.
        if req.offset != offset {
            if let Err(err) = snapshot.as_mut().seek(SeekFrom::Start(req.offset)).await {
                self.snapshot_state = Some(SnapshotState::Streaming { offset, id, snapshot });
                return Err(err.into());
            }
            offset = req.offset;
        }

        // Write the next segment & update offset.
        if let Err(err) = snapshot.as_mut().write_all(&req.data).await {
            self.snapshot_state = Some(SnapshotState::Streaming { offset, id, snapshot });
            return Err(err.into());
        }
        offset += req.data.len() as u64;

        // If the snapshot stream is done, then finalize.
        if req.done {
            self.finalize_snapshot_installation(req, snapshot).await?;
        } else {
            self.snapshot_state = Some(SnapshotState::Streaming { offset, id, snapshot });
        }
        Ok(InstallSnapshotResponse {
            term: self.current_term,
        })
    }

    /// Finalize the installation of a new snapshot.
    ///
    /// Any errors which come up from this routine will cause the Raft node to go into shutdown.
    #[tracing::instrument(level = "debug", skip(self, req, snapshot), fields(req=%req.summary()))]
    async fn finalize_snapshot_installation(
        &mut self,
        req: InstallSnapshotRequest,
        mut snapshot: Box<S::SnapshotData>,
    ) -> RaftResult<()> {
        snapshot.as_mut().shutdown().await.map_err(|err| self.map_fatal_storage_error(err.into()))?;

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

        // TODO(xp): do not install if self.last_applied >= snapshot.meta.last_applied

        let changes = self
            .storage
            .finalize_snapshot_installation(&req.meta, snapshot)
            .await
            .map_err(|e| self.map_storage_error(e))?;

        tracing::debug!("update after apply or install-snapshot: {:?}", changes);

        // After installing snapshot, no inconsistent log is removed.
        // This does not affect raft consistency.
        // If you have any question about this, let me know: drdr.xp at gmail.com

        if let Some(last_applied) = changes.last_applied {
            // Applied logs are not needed.
            delete_applied_logs(self.storage.clone(), &last_applied, self.config.max_applied_log_to_keep)
                .await
                .map_err(|e| self.map_storage_error(e))?;

            // snapshot is installed
            self.last_applied = last_applied;

            if self.committed < self.last_applied {
                self.committed = self.last_applied;
            }
            if self.last_log_id < self.last_applied {
                self.last_log_id = self.last_applied;
            }

            // There could be unknown membership in the snapshot.
            let membership = self.storage.get_membership_config().await.map_err(|err| self.map_storage_error(err))?;
            tracing::debug!("storage membership: {:?}", membership);

            self.update_membership(membership)?;

            self.snapshot_last_log_id = self.last_applied;
            self.report_metrics(Update::Ignore);
        } else {
            // snapshot not installed
        }

        Ok(())
    }
}
