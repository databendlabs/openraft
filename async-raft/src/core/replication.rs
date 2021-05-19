use tokio::sync::oneshot;

use crate::config::SnapshotPolicy;
use crate::core::{ConsensusState, LeaderState, ReplicationState, SnapshotState, State, UpdateCurrentLeader};
use crate::error::RaftResult;
use crate::replication::{RaftEvent, ReplicaEvent, ReplicationStream};
use crate::storage::CurrentSnapshotData;
use crate::{AppData, AppDataResponse, NodeId, RaftNetwork, RaftStorage};

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> LeaderState<'a, D, R, N, S> {
    /// Spawn a new replication stream returning its replication state handle.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) fn spawn_replication_stream(&self, target: NodeId) -> ReplicationState<D> {
        let replstream = ReplicationStream::new(
            self.core.id,
            target,
            self.core.current_term,
            self.core.config.clone(),
            self.core.last_log_index,
            self.core.last_log_term,
            self.core.commit_index,
            self.core.network.clone(),
            self.core.storage.clone(),
            self.replicationtx.clone(),
        );
        ReplicationState {
            match_index: self.core.last_log_index,
            match_term: self.core.current_term,
            is_at_line_rate: false,
            replstream,
            remove_after_commit: None,
        }
    }

    /// Handle a replication event coming from one of the replication streams.
    #[tracing::instrument(level = "trace", skip(self, event))]
    pub(super) async fn handle_replica_event(&mut self, event: ReplicaEvent<S::Snapshot>) {
        let res = match event {
            ReplicaEvent::RateUpdate { target, is_line_rate } => self.handle_rate_update(target, is_line_rate).await,
            ReplicaEvent::RevertToFollower { target, term } => self.handle_revert_to_follower(target, term).await,
            ReplicaEvent::UpdateMatchIndex {
                target,
                match_index,
                match_term,
            } => self.handle_update_match_index(target, match_index, match_term).await,
            ReplicaEvent::NeedsSnapshot { target, tx } => self.handle_needs_snapshot(target, tx).await,
            ReplicaEvent::Shutdown => {
                self.core.set_target_state(State::Shutdown);
                return;
            }
        };
        if let Err(err) = res {
            tracing::error!({error=%err}, "error while processing event from replication stream");
        }
    }

    /// Handle events from replication streams updating their replication rate tracker.
    #[tracing::instrument(level = "trace", skip(self, target, is_line_rate))]
    async fn handle_rate_update(&mut self, target: NodeId, is_line_rate: bool) -> RaftResult<()> {
        // Get a handle the target's replication stat & update it as needed.
        if let Some(state) = self.nodes.get_mut(&target) {
            state.is_at_line_rate = is_line_rate;
            return Ok(());
        }
        // Else, if this is a non-voter, then update as needed.
        if let Some(state) = self.non_voters.get_mut(&target) {
            state.state.is_at_line_rate = is_line_rate;
            state.is_ready_to_join = is_line_rate;
            // Issue a response on the non-voters response channel if needed.
            if state.is_ready_to_join {
                if let Some(tx) = state.tx.take() {
                    let _ = tx.send(Ok(()));
                }
                // If we are in NonVoterSync state, and this is one of the nodes being awaiting, then update.
                match std::mem::replace(&mut self.consensus_state, ConsensusState::Uniform) {
                    ConsensusState::NonVoterSync { mut awaiting, members, tx } => {
                        awaiting.remove(&target);
                        if awaiting.is_empty() {
                            // We are ready to move forward with entering joint consensus.
                            self.consensus_state = ConsensusState::Uniform;
                            self.change_membership(members, tx).await;
                        } else {
                            // We are still awaiting additional nodes, so replace our original state.
                            self.consensus_state = ConsensusState::NonVoterSync { awaiting, members, tx };
                        }
                    }
                    other => self.consensus_state = other, // Set the original value back to what it was.
                }
            }
        }
        Ok(())
    }

    /// Handle events from replication streams for when this node needs to revert to follower state.
    #[tracing::instrument(level = "trace", skip(self, term))]
    async fn handle_revert_to_follower(&mut self, _: NodeId, term: u64) -> RaftResult<()> {
        if term > self.core.current_term {
            self.core.update_current_term(term, None);
            self.core.save_hard_state().await?;
            self.core.update_current_leader(UpdateCurrentLeader::Unknown);
            self.core.set_target_state(State::Follower);
        }
        Ok(())
    }

    /// Handle events from a replication stream which updates the target node's match index.
    #[tracing::instrument(level = "trace", skip(self, target, match_index))]
    async fn handle_update_match_index(&mut self, target: NodeId, match_index: u64, match_term: u64) -> RaftResult<()> {
        // If this is a non-voter, then update and return.
        if let Some(state) = self.non_voters.get_mut(&target) {
            state.state.match_index = match_index;
            state.state.match_term = match_term;
            return Ok(());
        }

        // Update target's match index & check if it is awaiting removal.
        let mut needs_removal = false;
        match self.nodes.get_mut(&target) {
            Some(state) => {
                state.match_index = match_index;
                state.match_term = match_term;
                if let Some(threshold) = &state.remove_after_commit {
                    if &match_index >= threshold {
                        needs_removal = true;
                    }
                }
            }
            _ => return Ok(()), // Node not found.
        }

        // Drop replication stream if needed.
        if needs_removal {
            if let Some(node) = self.nodes.remove(&target) {
                let _ = node.replstream.repltx.send(RaftEvent::Terminate);
            }
        }

        // Determine the new commit index of the current membership config nodes.
        let mut indices_c0 = self
            .nodes
            .iter()
            .filter(|(id, _)| self.core.membership.members.contains(id))
            .map(|(_, node)| (node.match_index, node.match_term))
            .collect::<Vec<_>>();
        if !self.is_stepping_down {
            indices_c0.push((self.core.last_log_index, self.core.last_log_term));
        }
        let commit_index_c0 = calculate_new_commit_index(indices_c0, self.core.commit_index, self.core.current_term);

        // If we are in joint consensus, then calculate the new commit index of the new membership config nodes.
        let mut commit_index_c1 = commit_index_c0; // Defaults to just matching C0.
        if let Some(members) = &self.core.membership.members_after_consensus {
            let indices_c1 = self
                .nodes
                .iter()
                .filter(|(id, _)| members.contains(id))
                .map(|(_, node)| (node.match_index, node.match_term))
                .collect();
            commit_index_c1 = calculate_new_commit_index(indices_c1, self.core.commit_index, self.core.current_term);
        }

        // Determine if we have a new commit index, accounting for joint consensus.
        // If a new commit index has been established, then update a few needed elements.
        let has_new_commit_index = commit_index_c0 > self.core.commit_index && commit_index_c1 > self.core.commit_index;
        if has_new_commit_index {
            self.core.commit_index = std::cmp::min(commit_index_c0, commit_index_c1);

            // Update all replication streams based on new commit index.
            for node in self.nodes.values() {
                let _ = node.replstream.repltx.send(RaftEvent::UpdateCommitIndex {
                    commit_index: self.core.commit_index,
                });
            }
            for node in self.non_voters.values() {
                let _ = node.state.replstream.repltx.send(RaftEvent::UpdateCommitIndex {
                    commit_index: self.core.commit_index,
                });
            }

            // Check if there are any pending requests which need to be processed.
            let filter = self
                .awaiting_committed
                .iter()
                .enumerate()
                .take_while(|(_idx, elem)| elem.entry.index <= self.core.commit_index)
                .last()
                .map(|(idx, _)| idx);
            if let Some(offset) = filter {
                // Build a new ApplyLogsTask from each of the given client requests.
                for request in self.awaiting_committed.drain(..=offset).collect::<Vec<_>>() {
                    self.client_request_post_commit(request).await;
                }
            }
            self.core.report_metrics();
        }
        Ok(())
    }

    /// Handle events from replication streams requesting for snapshot info.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    async fn handle_needs_snapshot(&mut self, _: NodeId, tx: oneshot::Sender<CurrentSnapshotData<S::Snapshot>>) -> RaftResult<()> {
        // Ensure snapshotting is configured, else do nothing.
        let threshold = match &self.core.config.snapshot_policy {
            SnapshotPolicy::LogsSinceLast(threshold) => *threshold,
        };

        // Check for existence of current snapshot.
        let current_snapshot_opt = self
            .core
            .storage
            .get_current_snapshot()
            .await
            .map_err(|err| self.core.map_fatal_storage_error(err))?;
        if let Some(snapshot) = current_snapshot_opt {
            // If snapshot exists, ensure its distance from the leader's last log index is <= half
            // of the configured snapshot threshold, else create a new snapshot.
            if snapshot_is_within_half_of_threshold(&snapshot.index, &self.core.last_log_index, &threshold) {
                let _ = tx.send(snapshot);
                return Ok(());
            }
        }

        // Check if snapshot creation is already in progress. If so, we spawn a task to await its
        // completion (or cancellation), and respond to the replication stream. The repl stream
        // will wait for the completion and will then send anothe request to fetch the finished snapshot.
        // Else we just drop any other state and continue. Leaders never enter `Streaming` state.
        if let Some(SnapshotState::Snapshotting { handle, sender }) = self.core.snapshot_state.take() {
            let mut chan = sender.subscribe();
            tokio::spawn(async move {
                let _ = chan.recv().await;
                drop(tx);
            });
            self.core.snapshot_state = Some(SnapshotState::Snapshotting { handle, sender });
            return Ok(());
        }

        // At this point, we just attempt to request a snapshot. Under normal circumstances, the
        // leader will always be keeping up-to-date with its snapshotting, and the latest snapshot
        // will always be found and this block will never even be executed.
        //
        // If this block is executed, and a snapshot is needed, the repl stream will submit another
        // request here shortly, and will hit the above logic where it will await the snapshot completion.
        self.core.trigger_log_compaction_if_needed();
        Ok(())
    }
}

/// Determine the value for `current_commit` based on all known indicies of the cluster members.
///
/// - `entries`: is a vector of all of the highest known indices and terms to be replicated on a target node,
/// one per node of the cluster, including the leader as long as the leader is not stepping down.
/// - `current_commit`: is the Raft node's `current_commit` value before invoking this function.
/// The output of this function will never be less than this value.
/// - `leader_term`: the current leader term, only log entries from the leader’s current term are committed
/// by counting replicas.
///
/// NOTE: there are a few edge cases accounted for in this routine which will never practically
/// be hit, but they are accounted for in the name of good measure.
fn calculate_new_commit_index(mut entries: Vec<(u64, u64)>, current_commit: u64, leader_term: u64) -> u64 {
    if entries.is_empty() {
        return current_commit;
    }
    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let offset = (entries.len() + 1) / 2 - 1;
    let new_val = entries.get(offset).cloned().unwrap_or((current_commit, leader_term));
    if new_val.0 > current_commit && new_val.1 == leader_term {
        new_val.0
    } else {
        current_commit
    }
}

/// Check if the given snapshot data is within half of the configured threshold.
fn snapshot_is_within_half_of_threshold(snapshot_last_index: &u64, last_log_index: &u64, threshold: &u64) -> bool {
    // Calculate distance from actor's last log index.
    let distance_from_line = if snapshot_last_index > last_log_index {
        0u64
    } else {
        last_log_index - snapshot_last_index
    }; // Guard against underflow.
    let half_of_threshold = threshold / 2;
    distance_from_line <= half_of_threshold
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    //////////////////////////////////////////////////////////////////////////
    // snapshot_is_within_half_of_threshold //////////////////////////////////

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

    //////////////////////////////////////////////////////////////////////////
    // calculate_new_commit_index ////////////////////////////////////////////

    mod calculate_new_commit_index {
        use super::*;

        macro_rules! test_calculate_new_commit_index {
            ($name:ident, $expected:literal, $current:literal, $leader_term:literal, $entries:expr) => {
                #[test]
                fn $name() {
                    let mut entries = $entries;
                    let output = calculate_new_commit_index(entries.clone(), $current, $leader_term);
                    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                    assert_eq!(output, $expected, "Sorted values: {:?}", entries);
                }
            };
        }

        test_calculate_new_commit_index!(basic_values, 10, 5, 3, vec![(20, 3), (5, 2), (0, 2), (15, 3), (10, 3)]);

        test_calculate_new_commit_index!(len_zero_should_return_current_commit, 20, 20, 10, vec![]);

        test_calculate_new_commit_index!(len_one_where_greater_than_current, 100, 0, 3, vec![(100, 3)]);

        test_calculate_new_commit_index!(len_one_where_greater_than_current_but_smaller_term, 0, 0, 3, vec![(100, 2)]);

        test_calculate_new_commit_index!(len_one_where_less_than_current, 100, 100, 3, vec![(50, 3)]);

        test_calculate_new_commit_index!(even_number_of_nodes, 0, 0, 3, vec![(0, 3), (100, 3), (0, 3), (100, 3), (0, 3), (100, 3)]);

        test_calculate_new_commit_index!(
            majority_wins,
            100,
            0,
            3,
            vec![(0, 3), (100, 3), (0, 3), (100, 3), (0, 3), (100, 3), (100, 3)]
        );

        test_calculate_new_commit_index!(
            majority_entries_wins_but_not_current_term,
            0,
            0,
            3,
            vec![(0, 2), (100, 2), (0, 2), (101, 3), (0, 2), (101, 3), (101, 3)]
        );
    }
}
