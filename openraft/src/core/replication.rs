use std::collections::BTreeMap;

use tokio::sync::oneshot;
use tracing_futures::Instrument;

use crate::config::SnapshotPolicy;
use crate::core::LeaderState;
use crate::core::ReplicationState;
use crate::core::ServerState;
use crate::core::SnapshotState;
use crate::error::AddLearnerError;
use crate::metrics::UpdateMatchedLogId;
use crate::raft::AddLearnerResponse;
use crate::raft::RaftRespTx;
use crate::replication::RaftEvent;
use crate::replication::ReplicaEvent;
use crate::replication::ReplicationStream;
use crate::storage::Snapshot;
use crate::summary::MessageSummary;
use crate::versioned::Updatable;
use crate::vote::Vote;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LeaderState<'a, C, N, S> {
    /// Spawn a new replication stream returning its replication state handle.
    #[tracing::instrument(level = "debug", skip(self, caller_tx))]
    #[allow(clippy::type_complexity)]
    pub(super) async fn spawn_replication_stream(
        &mut self,
        target: C::NodeId,
        caller_tx: Option<RaftRespTx<AddLearnerResponse<C::NodeId>, AddLearnerError<C::NodeId>>>,
    ) -> ReplicationState<C::NodeId> {
        let target_node = self.core.engine.state.membership_state.effective.get_node(&target);
        let repl_stream = ReplicationStream::new::<C, N, S>(
            target,
            target_node.cloned(),
            self.core.engine.state.vote,
            self.core.config.clone(),
            self.core.engine.state.last_log_id,
            self.core.engine.state.committed,
            self.core.network.connect(target, target_node).await,
            self.core.storage.get_log_reader().await,
            self.replication_tx.clone(),
        );
        ReplicationState {
            matched: None,
            repl_stream,
            remove_since: None,
            tx: caller_tx,
        }
    }

    /// Handle a replication event coming from one of the replication streams.
    #[tracing::instrument(level = "trace", skip(self, event), fields(event=%event.summary()))]
    pub(super) async fn handle_replica_event(
        &mut self,
        event: ReplicaEvent<C::NodeId, S::SnapshotData>,
    ) -> Result<(), StorageError<C::NodeId>> {
        match event {
            ReplicaEvent::RevertToFollower { target, vote } => {
                self.handle_revert_to_follower(target, vote).await?;
            }
            ReplicaEvent::UpdateMatched { target, matched } => {
                self.handle_update_matched(target, matched).await?;
            }
            ReplicaEvent::NeedsSnapshot {
                target: _,
                must_include,
                tx,
            } => {
                self.handle_needs_snapshot(must_include, tx).await?;
            }
            ReplicaEvent::Shutdown => {
                self.core.set_target_state(ServerState::Shutdown);
            }
        };

        Ok(())
    }

    /// Handle events from replication streams for when this node needs to revert to follower state.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn handle_revert_to_follower(
        &mut self,
        _: C::NodeId,
        vote: Vote<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        if vote > self.core.engine.state.vote {
            self.core.engine.state.vote = vote;
            self.core.save_vote().await?;
            self.core.set_target_state(ServerState::Follower);
        }
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn handle_update_matched(
        &mut self,
        target: C::NodeId,
        matched: LogId<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        // Update target's match index & check if it is awaiting removal.

        if let Some(state) = self.nodes.get_mut(&target) {
            tracing::debug!("state.matched: {:?}, update to matched: {:?}", state.matched, matched);

            assert!(Some(matched) >= state.matched, "the matched increments monotonically");

            state.matched = Some(matched);

            // Issue a response on the learners response channel if needed.
            if state.is_line_rate(&self.core.engine.state.last_log_id, &self.core.config) {
                // This replication became line rate.

                // When adding a learner, it blocks until the replication becomes line-rate.
                if let Some(tx) = state.tx.take() {
                    // TODO(xp): define a specific response type for learner matched event.
                    let x = AddLearnerResponse { matched: state.matched };
                    let _ = tx.send(Ok(x));
                }
            }
        } else {
            return Ok(());
        }

        // Drop replication stream if needed.
        if self.try_remove_replication(target) {
            // nothing to do
        } else {
            self.update_replication_metrics(target, matched);
        }

        if Some(matched) <= self.core.engine.state.committed {
            return Ok(());
        }

        let commit_log_id = self.calc_commit_log_id();

        // Determine if we have a new commit index, accounting for joint consensus.
        // If a new commit index has been established, then update a few needed elements.

        if commit_log_id > self.core.engine.state.committed {
            self.core.engine.state.committed = commit_log_id;

            // Update all replication streams based on new commit index.
            for node in self.nodes.values() {
                let _ = node.repl_stream.repl_tx.send((
                    RaftEvent::UpdateCommittedLogId {
                        committed: self.core.engine.state.committed,
                    },
                    tracing::debug_span!("CH"),
                ));
            }

            // Apply committed entries, and send applying result to client if there is a channel awaiting it
            for i in self.core.engine.state.last_applied.next_index()..self.core.engine.state.committed.next_index() {
                self.client_request_post_commit(i).await?;
            }

            self.core.engine.metrics_flags.set_data_changed();
        }

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn update_replication_metrics(&mut self, target: C::NodeId, matched: LogId<C::NodeId>) {
        tracing::debug!(%target, ?matched, "update_leader_metrics");

        self.replication_metrics.update(UpdateMatchedLogId { target, matched });
        self.core.engine.metrics_flags.set_replication_changed()
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn calc_commit_log_id(&self) -> Option<LogId<C::NodeId>> {
        let repl_indexes = self.get_match_log_ids();

        let committed =
            self.core.engine.state.membership_state.effective.membership.greatest_majority_value(&repl_indexes);

        // TODO(xp): remove this line
        std::cmp::max(committed.cloned(), self.core.engine.state.committed)

        // *committed.unwrap_or(&self.core.log_store.st.committed)
    }

    /// Collect indexes of the greatest matching log on every replica(include the leader itself)
    fn get_match_log_ids(&self) -> BTreeMap<C::NodeId, LogId<C::NodeId>> {
        let node_ids = self.core.engine.state.membership_state.effective.all_members();

        let mut res = BTreeMap::new();

        for id in node_ids.iter() {
            let matched = if *id == self.core.id {
                self.core.engine.state.last_log_id
            } else {
                let repl_state = self.nodes.get(id);
                repl_state.map(|x| x.matched).unwrap_or_default()
            };

            // Mismatching term can not prevent other replica with higher term log from being chosen as leader,
            // and that new leader may overrides any lower term logs.
            // Thus it is not considered as committed.
            if let Some(log_id) = matched {
                if log_id.leader_id == self.core.engine.state.vote.leader_id() {
                    res.insert(*id, log_id);
                }
            }
        }

        res
    }

    /// A replication streams requesting for snapshot info.
    ///
    /// The snapshot has to include `must_include`.
    #[tracing::instrument(level = "debug", skip(self, tx))]
    async fn handle_needs_snapshot(
        &mut self,
        must_include: Option<LogId<C::NodeId>>,
        tx: oneshot::Sender<Snapshot<C::NodeId, S::SnapshotData>>,
    ) -> Result<(), StorageError<C::NodeId>> {
        // Ensure snapshotting is configured, else do nothing.
        let threshold = match &self.core.config.snapshot_policy {
            SnapshotPolicy::LogsSinceLast(threshold) => *threshold,
        };

        // Check for existence of current snapshot.
        let current_snapshot_opt = self.core.storage.get_current_snapshot().await?;

        if let Some(snapshot) = current_snapshot_opt {
            if let Some(must_inc) = must_include {
                if snapshot.meta.last_log_id >= must_inc {
                    let _ = tx.send(snapshot);
                    return Ok(());
                }
            } else {
                // If snapshot exists, ensure its distance from the leader's last log index is <= half
                // of the configured snapshot threshold, else create a new snapshot.
                if snapshot_is_within_half_of_threshold(
                    &snapshot.meta.last_log_id.index,
                    &self.core.engine.state.last_log_id.unwrap_or_default().index,
                    &threshold,
                ) {
                    let _ = tx.send(snapshot);
                    return Ok(());
                }
            }
        }

        // Check if snapshot creation is already in progress. If so, we spawn a task to await its
        // completion (or cancellation), and respond to the replication stream. The repl stream
        // will wait for the completion and will then send another request to fetch the finished snapshot.
        // Else we just drop any other state and continue. Leaders never enter `Streaming` state.
        if let Some(SnapshotState::Snapshotting { handle, sender }) = self.core.snapshot_state.take() {
            let mut chan = sender.subscribe();
            tokio::spawn(
                async move {
                    let _ = chan.recv().await;
                    // TODO(xp): send another ReplicaEvent::NeedSnapshot to raft core
                    drop(tx);
                }
                .instrument(tracing::debug_span!("spawn-recv-and-drop")),
            );
            self.core.snapshot_state = Some(SnapshotState::Snapshotting { handle, sender });
            return Ok(());
        }

        // At this point, we just attempt to request a snapshot. Under normal circumstances, the
        // leader will always be keeping up-to-date with its snapshotting, and the latest snapshot
        // will always be found and this block will never even be executed.
        //
        // If this block is executed, and a snapshot is needed, the repl stream will submit another
        // request here shortly, and will hit the above logic where it will await the snapshot completion.
        //
        // If snapshot is too old, i.e., the distance from last_log_index is greater than half of snapshot threshold,
        // always force a snapshot creation.
        self.core.trigger_log_compaction_if_needed(true).await;
        Ok(())
    }
}

/// Check if the given snapshot data is within half of the configured threshold.
fn snapshot_is_within_half_of_threshold(snapshot_last_index: &u64, last_log_index: &u64, threshold: &u64) -> bool {
    // Calculate distance from actor's last log index.
    let distance_from_line = last_log_index.saturating_sub(*snapshot_last_index);

    distance_from_line <= threshold / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    mod snapshot_is_within_half_of_threshold {
        use super::*;

        macro_rules! test_snapshot_is_within_half_of_threshold {
            ({test=>$name:ident, snapshot_last_index=>$snapshot_last_index:expr, last_log_index=>$last_log:expr, threshold=>$thresh:expr, expected=>$exp:literal}) => {
                #[test]
                fn $name() {
                    let res = snapshot_is_within_half_of_threshold($snapshot_last_index, $last_log, $thresh);
                    assert_eq!(res, $exp)
                }
            };
        }

        test_snapshot_is_within_half_of_threshold!({
            test=>happy_path_true_when_within_half_threshold,
            snapshot_last_index=>&50, last_log_index=>&100, threshold=>&500, expected=>true
        });

        test_snapshot_is_within_half_of_threshold!({
            test=>happy_path_false_when_above_half_threshold,
            snapshot_last_index=>&1, last_log_index=>&500, threshold=>&100, expected=>false
        });

        test_snapshot_is_within_half_of_threshold!({
            test=>guards_against_underflow,
            snapshot_last_index=>&200, last_log_index=>&100, threshold=>&500, expected=>true
        });
    }
}
