use actix::prelude::*;
use log::{error, warn};

use crate::{
    AppError,
    common::{CLIENT_RPC_TX_ERR, ApplyLogsTask, DependencyAddr, UpdateCurrentLeader},
    config::SnapshotPolicy,
    messages::{ClientPayloadResponse, ResponseMode},
    network::RaftNetwork,
    raft::{Raft, RaftState},
    replication::{
        RSFatalActixMessagingError, RSFatalStorageError,
        RSNeedsSnapshot, RSNeedsSnapshotResponse,
        RSRateUpdate, RSUpdateLineCommit, RSRevertToFollower, RSUpdateMatchIndex,
    },
    storage::{CreateSnapshot, GetCurrentSnapshot, CurrentSnapshotData, RaftStorage},
};

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSFatalActixMessagingError ////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSFatalActixMessagingError> for Raft<E, N, S> {
    type Result = ();

    /// Handle events from replication streams reporting errors.
    fn handle(&mut self, msg: RSFatalActixMessagingError, ctx: &mut Self::Context) {
        self.map_fatal_actix_messaging_error(ctx, msg.err, msg.dependency);
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSFatalStorageError ///////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSFatalStorageError<E>> for Raft<E, N, S> {
    type Result = ();

    /// Handle events from replication streams reporting errors.
    fn handle(&mut self, msg: RSFatalStorageError<E>, ctx: &mut Self::Context) {
        let err: Result<(), E> = Err(msg.err);
        let _ = self.map_fatal_storage_result(ctx, err);
    }
}

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
                warn!("Received an RSNeedsSnapshot request from a replication stream, but snapshotting is disabled. Cluster is misconfigured.");
                return Box::new(fut::err(()));
            }
        };

        // Check for existence of current snapshot.
        Box::new(fut::wrap_future(self.storage.send(GetCurrentSnapshot::new()))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
            .and_then(move |res, act, _| {
                if let Some(meta) = res {
                    // If snapshot exists, ensure its distance from the leader's last log index is <= half
                    // of the configured snapshot threshold, else create a new snapshot.
                    if snapshot_is_within_half_of_threshold(&meta, act.last_log_index, threshold) {
                        let CurrentSnapshotData{index, term, config, pointer} = meta;
                        return fut::Either::A(fut::ok(RSNeedsSnapshotResponse{index, term, config, pointer}));
                    }
                }
                // If snapshot is not within half of threshold, or if snapshot does not exist, create a new snapshot.
                // Create a new snapshot up through the committed index (to avoid jitter).
                fut::Either::B(fut::wrap_future(act.storage.send(CreateSnapshot::new(act.commit_index)))
                    .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
                    .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
                    .and_then(|res, _, _| {
                        let CurrentSnapshotData{index, term, config, pointer} = res;
                        fut::ok(RSNeedsSnapshotResponse{index, term, config, pointer})
                    }))
            }))
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSRevertToFollower ////////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSRevertToFollower> for Raft<E, N, S> {
    type Result = ();

    /// Handle events from replication streams for when this node needs to revert to follower state.
    fn handle(&mut self, msg: RSRevertToFollower, ctx: &mut Self::Context) {
        if &msg.term > &self.current_term {
            self.update_current_term(msg.term, None);
            self.update_current_leader(ctx, UpdateCurrentLeader::Unknown);
            self.become_follower(ctx);
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSUpdateMatchIndex ////////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSUpdateMatchIndex> for Raft<E, N, S> {
    type Result = ();

    /// Handle events from a replication stream which updates the target node's match index.
    fn handle(&mut self, msg: RSUpdateMatchIndex, _: &mut Self::Context) {
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
        let new_commit_index = calculate_new_commit_index(indices, self.commit_index);
        let has_new_commit_index = new_commit_index > self.commit_index;

        // If a new commit index has been determined, update a few needed elements.
        if has_new_commit_index {
            self.commit_index = new_commit_index;

            // Update all replication streams based on new commit index.
            for node in state.nodes.iter() {
                let _ = node.1.addr.do_send(RSUpdateLineCommit(self.commit_index));
            }

            // Check if there are any pending requests which need to be processed.
            let filter = state.awaiting_committed.iter().enumerate()
                .take_while(|(_idx, elem)| &elem.index <= &new_commit_index).last()
                .map(|(idx, _)| idx);
            if let Some(offset) = filter {
                // Build a new ApplyLogsTask from each of the given client requests.
                for request in state.awaiting_committed.drain(..=offset) {
                    if let &ResponseMode::Committed = &request.response_mode {
                        // If this RPC is configured to wait only for log committed, then respond to client now.
                        let entry = request.entry();
                        let _ = request.tx.send(Ok(ClientPayloadResponse{index: request.index})).map_err(|err| error!("{} {:?}", CLIENT_RPC_TX_ERR, err));
                        let _ = self.apply_logs_pipeline.unbounded_send(ApplyLogsTask::Entry{entry, chan: None});
                    } else {
                        // Else, send it through the pipeline and it will be responded to afterwords.
                        let _ = self.apply_logs_pipeline.unbounded_send(ApplyLogsTask::Entry{entry: request.entry(), chan: Some(request.tx)});
                    }
                }
            }
        }
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
fn snapshot_is_within_half_of_threshold(data: &CurrentSnapshotData, last_log_index: u64, threshold: u64) -> bool {
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
            data=>&CurrentSnapshotData{term: 1, index: 50, config: vec![], pointer: EntrySnapshotPointer{path: String::new()}},
            last_log_index=>100, threshold=>500, expected=>true
        });

        test_snapshot_is_within_half_of_threshold!({
            test=>happy_path_false_when_above_half_threshold,
            data=>&CurrentSnapshotData{term: 1, index: 1, config: vec![], pointer: EntrySnapshotPointer{path: String::new()}},
            last_log_index=>500, threshold=>100, expected=>false
        });

        test_snapshot_is_within_half_of_threshold!({
            test=>guards_against_underflow,
            data=>&CurrentSnapshotData{term: 1, index: 200, config: vec![], pointer: EntrySnapshotPointer{path: String::new()}},
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
