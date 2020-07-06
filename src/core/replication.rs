use tokio::sync::oneshot;

use crate::{AppData, AppDataResponse, AppError, NodeId, RaftNetwork, RaftStorage};
use crate::config::SnapshotPolicy;
use crate::error::RaftResult;
use crate::core::{ConsensusState, LeaderState, SnapshotState, TargetState, UpdateCurrentLeader};
use crate::replication::{RaftEvent, ReplicaEvent};
use crate::storage::CurrentSnapshotData;

impl<'a, D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> LeaderState<'a, D, R, E, N, S> {
    pub(super) async fn handle_replica_event(&mut self, event: ReplicaEvent) {
        let res = match event {
            ReplicaEvent::RateUpdate{target, is_line_rate} => self.handle_rate_update(target, is_line_rate).await,
            ReplicaEvent::RevertToFollower{target, term} => self.handle_revert_to_follower(target, term).await,
            ReplicaEvent::UpdateMatchIndex{target, match_index} => self.handle_update_match_index(target, match_index).await,
            ReplicaEvent::NeedsSnapshot{target, tx} => self.handle_needs_snapshot(target, tx).await,
        };
        if let Err(err) = res {
            tracing::error!({error=%err}, "error while processing event from replication stream");
        }
    }

    /// Handle events from replication streams updating their replication rate tracker.
    async fn handle_rate_update(&mut self, target: NodeId, is_line_rate: bool) -> RaftResult<(), E> {
        // Get a handle the target's replication stat & update it as needed.
        let repl_state = match self.nodes.get_mut(&target) {
            Some(repl_state) => repl_state,
            _ => return Ok(()),
        };
        repl_state.is_at_line_rate = is_line_rate;
        // If in joint consensus, and the target node was one of the new nodes, update
        // the joint consensus state to indicate that the target is up-to-date.
        if let ConsensusState::Joint{new_nodes_being_synced, ..} = &mut self.consensus_state {
            if let Some((idx, _)) = new_nodes_being_synced.iter().enumerate().find(|(_, e)| e == &&target) {
                new_nodes_being_synced.remove(idx);
            }
            // If there are no remaining nodes to sync, then finalize this joint consensus.
            if self.consensus_state.is_joint_consensus_safe_to_finalize() {
                self.finalize_joint_consensus().await?;
            }
        }

        Ok(())
    }

    /// Handle events from replication streams for when this node needs to revert to follower state.
    async fn handle_revert_to_follower(&mut self, _: NodeId, term: u64) -> RaftResult<(), E> {
        if &term > &self.core.current_term {
            self.core.update_current_term(term, None);
            self.core.save_hard_state().await?;
            self.core.update_current_leader(UpdateCurrentLeader::Unknown);
            self.core.set_target_state(TargetState::Follower);
        }
        Ok(())
    }

    /// Handle events from a replication stream which updates the target node's match index.
    #[tracing::instrument(skip(self))]
    async fn handle_update_match_index(&mut self, target: NodeId, match_index: u64) -> RaftResult<(), E> {
        // Update target's match index & check if it is awaiting removal.
        let mut needs_removal = false;
        match self.nodes.get_mut(&target) {
            Some(replstate) => {
                replstate.match_index = match_index;
                if let Some(threshold) = &replstate.remove_after_commit {
                    if &match_index >= threshold {
                        needs_removal = true;
                    }
                }
            },
            _ => return Ok(()), // Node not found.
        }

        // Drop replication stream if needed.
        if needs_removal {
            if let Some(node) = self.nodes.remove(&target) {
                let _ = node.replstream.repltx.send(RaftEvent::Terminate);
            }
        }

        // Parse through each targets' match index, and update the value of `commit_index` based
        // on the highest value which has been replicated to a majority of the cluster
        // including the leader which created the entry.
        let mut indices: Vec<_> = self.nodes.values().map(|elem| elem.match_index).collect();
        indices.push(self.core.last_log_index);
        let new_commit_index = calculate_new_commit_index(indices, self.core.commit_index);
        let has_new_commit_index = &new_commit_index > &self.core.commit_index;

        // If a new commit index has been determined, update a few needed elements.
        if has_new_commit_index {
            self.core.commit_index = new_commit_index;

            // Update all replication streams based on new commit index.
            for node in self.nodes.values() {
                let _ = node.replstream.repltx.send(RaftEvent::UpdateCommitIndex{commit_index: new_commit_index});
            }

            // Check if there are any pending requests which need to be processed.
            let filter = self.awaiting_committed.iter().enumerate()
                .take_while(|(_idx, elem)| &elem.entry.index <= &new_commit_index)
                .last()
                .map(|(idx, _)| idx);
            if let Some(offset) = filter {
                // Build a new ApplyLogsTask from each of the given client requests.
                for request in self.awaiting_committed.drain(..=offset).collect::<Vec<_>>() {
                    self.client_request_post_commit(request).await;
                }
            }
        }
        Ok(())
    }

    /// Handle events from replication streams requesting for snapshot info.
    async fn handle_needs_snapshot(&mut self, _: NodeId, tx: oneshot::Sender<CurrentSnapshotData>) -> RaftResult<(), E> {
        // Ensure snapshotting is configured, else do nothing.
        let threshold = match &self.core.config.snapshot_policy {
            SnapshotPolicy::LogsSinceLast(threshold) => *threshold,
        };

        // Check for existence of current snapshot.
        let current_snapshot_opt = self.core.storage.get_current_snapshot().await
            .map_err(|err| self.core.map_fatal_storage_result(err))?;
        if let Some(snapshot) = current_snapshot_opt {
            // If snapshot exists, ensure its distance from the leader's last log index is <= half
            // of the configured snapshot threshold, else create a new snapshot.
            if snapshot_is_within_half_of_threshold(&snapshot, &self.core.last_log_index, &threshold) {
                let _ = tx.send(snapshot);
                return Ok(());
            }
        }

        // Check if snapshot creation is already in progress.
        match self.core.snapshot_state.take() {
            // If so, we spawn a task to await its completion (or cancellation), and respond to
            // the replication stream. The repl stream will wait for the completion and will then
            // send anothe request to fetch the finished snapshot.
            Some(SnapshotState::Snapshotting{through, handle, sender}) => {
                let mut chan = sender.subscribe();
                tokio::spawn(async move {
                    let _ = chan.recv().await;
                    drop(tx);
                });
                self.core.snapshot_state = Some(SnapshotState::Snapshotting{through, handle, sender});
                return Ok(());
            }
            _ => (), // Else we just drop any other state and continue. Leaders never enter `Streaming` state.
        }

        // At this point, we just attempt to request a snapshot. Under normal circumstances, the
        // leader will always be keeping up-to-date with its snapshotting, and the latest snapshot
        // will always be found and this block will never even be executed.
        //
        // If this block is executed, and a snapshot is needed, the repl stream will submit another
        // request here shortly, and will hit the above logic where it will await the snapshot complection.
        self.core.trigger_log_compaction_if_needed();
        Ok(())
    }
}

/// Determine the value for `current_commit` based on all known indicies of the cluster members.
///
/// - `entries`: is a vector of all of the highest known index to be replicated on a target node,
/// one per node of the cluster, including the leader.
/// - `current_commit`: is the Raft node's `current_commit` value before invoking this function.
/// The output of this function will never be less than this value.
///
/// NOTE: there are a few edge cases accounted for in this routine which will never practically
/// be hit, but they are accounted for in the name of good measure.
fn calculate_new_commit_index(mut entries: Vec<u64>, current_commit: u64) -> u64 {
    // Handle cases where len < 2.
    let len = entries.len();
    if len == 0 {
        return current_commit;
    } else if len == 1 {
        let only_elem = entries[0];
        return if only_elem < current_commit { current_commit } else { only_elem };
    };

    // Calculate offset which will give the majority slice of high-end.
    entries.sort();
    let offset = if (len % 2) == 0 { (len/2)-1 } else { len/2 };
    let new_val = entries.get(offset).unwrap_or(&current_commit);
    if new_val < &current_commit {
        current_commit
    } else {
        *new_val
    }
}

/// Check if the given snapshot data is within half of the configured threshold.
fn snapshot_is_within_half_of_threshold(data: &CurrentSnapshotData, last_log_index: &u64, threshold: &u64) -> bool {
    // Calculate distance from actor's last log index.
    let distance_from_line = if &data.index > last_log_index { 0u64 } else { last_log_index - &data.index }; // Guard against underflow.
    let half_of_threshold = threshold / &2;
    distance_from_line <= half_of_threshold
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raft::MembershipConfig;
    use crate::storage::SnapshotReader;

    impl SnapshotReader for std::io::Cursor<Vec<u8>> {}

    //////////////////////////////////////////////////////////////////////////
    // snapshot_is_within_half_of_threshold //////////////////////////////////

    mod snapshot_is_within_half_of_threshold {
        use super::*;

        macro_rules! test_snapshot_is_within_half_of_threshold {
            ({test=>$name:ident, data=>$data:expr, last_log_index=>$last_log:expr, threshold=>$thresh:expr, expected=>$exp:literal}) => {
                #[test]
                fn $name() {
                    let res = snapshot_is_within_half_of_threshold($data, $last_log, $thresh);
                    assert_eq!(res, $exp)
                }
            }
        }

        test_snapshot_is_within_half_of_threshold!({
            test=>happy_path_true_when_within_half_threshold,
            data=>&CurrentSnapshotData{
                term: 1, index: 50, membership: MembershipConfig{members: vec![], non_voters: vec![], removing: vec![], is_in_joint_consensus: false},
                snapshot: Box::new(std::io::Cursor::new(vec![])),
            },
            last_log_index=>&100, threshold=>&500, expected=>true
        });

        test_snapshot_is_within_half_of_threshold!({
            test=>happy_path_false_when_above_half_threshold,
            data=>&CurrentSnapshotData{
                term: 1, index: 1, membership: MembershipConfig{members: vec![], non_voters: vec![], removing: vec![], is_in_joint_consensus: false},
                snapshot: Box::new(std::io::Cursor::new(vec![])),
            },
            last_log_index=>&500, threshold=>&100, expected=>false
        });

        test_snapshot_is_within_half_of_threshold!({
            test=>guards_against_underflow,
            data=>&CurrentSnapshotData{
                term: 1, index: 200, membership: MembershipConfig{members: vec![], non_voters: vec![], removing: vec![], is_in_joint_consensus: false},
                snapshot: Box::new(std::io::Cursor::new(vec![])),
            },
            last_log_index=>&100, threshold=>&500, expected=>true
        });
    }

    //////////////////////////////////////////////////////////////////////////
    // calculate_new_commit_index ////////////////////////////////////////////

    mod calculate_new_commit_index {
        use super::*;

        macro_rules! test_calculate_new_commit_index {
            ($name:ident, $expected:literal, $current:literal, $entries:expr) => {
                #[test]
                fn $name() {
                    let mut entries = $entries;
                    let output = calculate_new_commit_index(entries.clone(), $current);
                    entries.sort();
                    assert_eq!(output, $expected, "Sorted values: {:?}", entries);
                }
            }
        }

        test_calculate_new_commit_index!(
            basic_values,
            10, 5, vec![20, 5, 0, 15, 10]
        );

        test_calculate_new_commit_index!(
            len_zero_should_return_current_commit,
            20, 20, vec![]
        );

        test_calculate_new_commit_index!(
            len_one_where_greater_than_current,
            100, 0, vec![100]
        );

        test_calculate_new_commit_index!(
            len_one_where_less_than_current,
            100, 100, vec![50]
        );

        test_calculate_new_commit_index!(
            even_number_of_nodes,
            0, 0, vec![0, 100, 0, 100, 0, 100]
        );

        test_calculate_new_commit_index!(
            majority_wins,
            100, 0, vec![0, 100, 0, 100, 0, 100, 100]
        );
    }
}
