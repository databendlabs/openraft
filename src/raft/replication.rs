use actix::prelude::*;
use log::{warn};

use crate::{
    AppError,
    config::SnapshotPolicy,
    network::RaftNetwork,
    raft::{Raft, RaftState, common::{DependencyAddr, UpdateCurrentLeader}},
    replication::{
        RSNeedsSnapshot, RSNeedsSnapshotResponse,
        RSRateUpdate, RSRevertToFollower, RSUpdateMatchIndex,
    },
    storage::{GetCurrentSnapshot, GetCurrentSnapshotData, RaftStorage},
};

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSRateUpdate //////////////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSRateUpdate> for Raft<E, N, S> {
    type Result = ();

    /// Handle events from replication streams updating their replication rate tracker.
    fn handle(&mut self, msg: RSRateUpdate, _ctx: &mut Self::Context) {
        // Extract leader state, else do nothing.
        let state = match &mut self.state {
            RaftState::Leader(state) => state,
            _ => return,
        };

        // Get a handle the target's replication stat & update it as needed.
        match state.nodes.get_mut(&msg.target) {
            Some(repl_state) => {
                repl_state.is_at_line_rate = msg.is_line_rate;
            },
            _ => (),
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSNeedsSnapshot ///////////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSNeedsSnapshot> for Raft<E, N, S> {
    type Result = ResponseActFuture<Self, RSNeedsSnapshotResponse, ()>;

    /// Handle events from replication streams requesting for snapshot info.
    fn handle(&mut self, _: RSNeedsSnapshot, _ctx: &mut Self::Context) -> Self::Result {
        // Extract leader state, else do nothing.
        match &mut self.state {
            RaftState::Leader(_) => (),
            _ => return Box::new(fut::err(())),
        };

        // Ensure snapshotting is configured, else do nothing.
        let threshold = match &self.config.snapshot_policy {
            SnapshotPolicy::LogsSinceLast(threshold) => *threshold,
            SnapshotPolicy::Disabled => {
                warn!("Received an RSNeedsSnapshot request from a replication stream, but snapshotting is disabled. This is a bug.");
                return Box::new(fut::err(()));
            }
        };

        // Check for existence of current snapshot.
        let _ = fut::wrap_future(self.storage.send(GetCurrentSnapshot::new()))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
            .and_then(move |res, act, _| match res {
                // If snapshot exists, ensure its distance from the leader's last log index is <= half
                // of the configured snapshot threshold, else create a new snapshot.
                Some(meta) => if snapshot_is_within_half_of_threshold(meta, act.last_log_index, threshold) {
                    fut::ok(())
                } else {
                    fut::ok(())
                }
                // If snapshot does not exist, create a new snapshot.
                None => {
                    fut::ok(())
                }
            });

        // Box::new(fut::ok(RSNeedsSnapshotResponse {
        //     index: 0,
        //     term: 0,
        //     pointer: EntrySnapshotPointer,
        // }))

        Box::new(fut::err(()))
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSRevertToFollower ////////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSRevertToFollower> for Raft<E, N, S> {
    type Result = ();

    /// Handle events from replication streams for when this node needs to revert to follower state.
    fn handle(&mut self, _msg: RSRevertToFollower, ctx: &mut Self::Context) {
        self.update_current_leader(ctx, UpdateCurrentLeader::Unknown);
        self.become_follower(ctx);
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSUpdateMatchIndex ////////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSUpdateMatchIndex> for Raft<E, N, S> {
    type Result = ();

    /// Handle events from a replication stream which updates the target node's match index.
    fn handle(&mut self, msg: RSUpdateMatchIndex, _ctx: &mut Self::Context) {
        // Extract leader state, else do nothing.
        let state = match &mut self.state {
            RaftState::Leader(state) => state,
            _ => return,
        };

        // Update target's match index.
        match state.nodes.get_mut(&msg.target) {
            Some(repl_state) => {
                repl_state.match_index = msg.match_index;
            },
            _ => return,
        }

        // Parse through each targets' match index, and update the value of `commit_index` based
        // on the highest value which has been replicated to a majority of the cluster
        // including the leader which created the entry.
        let mut indices: Vec<_> = state.nodes.values().map(|elem| elem.match_index).collect();
        indices.push(self.last_log_index);
        self.commit_index = calculate_new_commit_index(indices, self.commit_index);
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
fn snapshot_is_within_half_of_threshold(data: GetCurrentSnapshotData, last_log_index: u64, threshold: u64) -> bool {
    // Calculate distance from actor's last log index.
    let distance_from_line = if data.index > last_log_index { 0u64 } else { last_log_index - data.index }; // Guard against underflow.
    let half_of_threshold = threshold / 2;
    distance_from_line <= half_of_threshold
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// Unit Tests ////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::EntrySnapshotPointer;

    //////////////////////////////////////////////////////////////////////////
    // snapshot_is_within_half_of_threshold //////////////////////////////////

    mod snapshot_is_within_half_of_threshold {
        use super::*;

        macro_rules! test_snapshot_is_within_half_of_threshold {
            ({test=>$name:ident, data=>$data:expr, last_log_index=>$last_log:literal, threshold=>$thresh:literal, expected=>$exp:literal}) => {
                #[test]
                fn $name() {
                    let res = snapshot_is_within_half_of_threshold($data, $last_log, $thresh);
                    assert_eq!(res, $exp)
                }
            }
        }

        test_snapshot_is_within_half_of_threshold!({
            test=>happy_path_true_when_within_half_threshold,
            data=>GetCurrentSnapshotData{term: 1, index: 50, pointer: EntrySnapshotPointer{path: String::new()}},
            last_log_index=>100, threshold=>500, expected=>true
        });

        test_snapshot_is_within_half_of_threshold!({
            test=>happy_path_false_when_above_half_threshold,
            data=>GetCurrentSnapshotData{term: 1, index: 1, pointer: EntrySnapshotPointer{path: String::new()}},
            last_log_index=>500, threshold=>100, expected=>false
        });

        test_snapshot_is_within_half_of_threshold!({
            test=>guards_against_underflow,
            data=>GetCurrentSnapshotData{term: 1, index: 200, pointer: EntrySnapshotPointer{path: String::new()}},
            last_log_index=>100, threshold=>500, expected=>true
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
