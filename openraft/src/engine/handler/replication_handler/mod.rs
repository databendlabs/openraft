use std::sync::Arc;

use display_more::DisplayOptionExt;
use display_more::DisplayResultExt;
use progress::entry::update::Updater;
use validit::Valid;

use crate::LogIdOptionExt;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::engine::TargetProgress;
use crate::engine::handler::log_handler::LogHandler;
use crate::errors::NodeNotFound;
use crate::errors::Operation;
use crate::membership::RaftMembership;
use crate::membership::RaftMembershipExt;
use crate::progress;
use crate::progress::IdVal;
use crate::progress::Inflight;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::proposer::Leader;
use crate::proposer::SendStamp;
use crate::proposer::WriteQuorumSet;
use crate::raft::ConflictHint;
use crate::raft_state::LogStateReader;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::replication::replicate::Replicate;
use crate::replication::response::Progress;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::CommittedVoteOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::vote::raft_vote::RaftVoteExt;

#[cfg(test)]
mod append_membership_test;
#[cfg(test)]
mod receive_logs_test;
#[cfg(test)]
mod try_purge_log_test;
#[cfg(test)]
mod update_conflicting_test;
#[cfg(test)]
mod update_matching_test;
#[cfg(test)]
mod update_progress_test;

/// Bounds replication attached to each matching-point probe, avoiding large fragmented ranges
/// while retaining batching for small follower gaps.
const MAX_ENTRIES_PER_PROBE: u64 = 8;

/// Handle replication operations.
///
/// - Writing local log store;
/// - Replicating log to a remote node;
/// - Tracking membership changes and update related state;
/// - Tracking replication progress and commit;
/// - Purging in-snapshot logs;
/// - etc.
pub(crate) struct ReplicationHandler<'x, C, SM = ()>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) config: &'x mut EngineConfig<C>,
    pub(crate) leader: &'x mut Leader<C, WriteQuorumSet<C>>,
    pub(crate) state: &'x mut RaftState<C>,
    pub(crate) output: &'x mut EngineOutput<C, SM>,
}

impl<'x, C, SM> ReplicationHandler<'x, C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(
        config: &'x mut EngineConfig<C>,
        leader: &'x mut Leader<C, WriteQuorumSet<C>>,
        state: &'x mut RaftState<C>,
        output: &'x mut EngineOutput<C, SM>,
    ) -> Self {
        Self {
            config,
            leader,
            state,
            output,
        }
    }

    /// Append a new membership and update related state such as replication streams.
    ///
    /// It is called by the leader when a new membership log is appended to the log store. It does
    /// not initiate replication: the caller that appended the log does, see
    /// `LeaderHandler::leader_append_entries()`. It retries a pending purge after the streams are
    /// rebuilt, because a target that it removes may have been the last to hold the purge back.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn append_membership(&mut self, log_id: &LogIdOf<C>, m: &C::Membership) {
        tracing::debug!("update effective membership: log_id:{} {}", log_id, m);

        let old_membership = self.state.membership_state.effective().clone();

        self.state
            .membership_state
            .append(StoredMembershipOf::<C>::new_arc(Some(log_id.clone()), m.clone()));

        // Leader does not quit at once:
        // A leader should always keep replicating logs.
        // A leader that is removed will shut down replications when this membership log is
        // committed.

        self.rebuild_progresses();
        self.renew_streams_of_changed_nodes(&old_membership);
        self.rebuild_replication_streams(false);

        // A removed target, or one that no longer receives logs, may have been the last to hold
        // back a pending purge, and no answer of its closed stream will retry it.
        self.try_purge_log();
    }

    /// Give a new replication stream to each target whose node metadata differs from `old`.
    ///
    /// A network client may capture the metadata it was created from, such as an address, so
    /// `RaftCore` replaces the stream of such a target instead of reusing it, see
    /// [`Command::RebuildReplicationStreams`]. The replaced stream never answers the target's
    /// inflight request, so the request is dropped, and [`Self::initiate_replication`] sends the
    /// next one through the new stream. The new stream id makes a late response from the replaced
    /// stream stale. The matching log id stays, because the target is the same node.
    fn renew_streams_of_changed_nodes(&mut self, old: &StoredMembershipOf<C>) {
        let new = self.state.membership_state.effective().clone();
        let id_gen = self.state.progress_id_gen.clone();

        for item in self.leader.progress.iter_mut_without_reorder() {
            if item.id == self.config.id {
                continue;
            }
            let (Some(old_node), Some(new_node)) = (old.get_node(&item.id), new.get_node(&item.id)) else {
                continue;
            };
            if old_node == new_node {
                continue;
            }

            tracing::info!(
                "node metadata of target {} changed from {:?} to {:?}: replace its replication stream",
                item.id,
                old_node,
                new_node
            );
            item.data.inflight = Inflight::None;
            item.data.stream_id = StreamId::new(id_gen.next_id());
        }
    }

    /// Rebuild leader's replication progress to reflect replication changes.
    ///
    /// E.g., when adding/removing a follower/learner.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn rebuild_progresses(&mut self) {
        let em = self.state.membership_state.effective();

        let non_write_voter_ids = em.non_write_voter_ids().collect::<Vec<_>>();
        let write_quorum_set = Arc::new(em.membership().write_quorum_set());

        {
            let end = self.state.last_log_id().next_index();
            let probe_start = self.leader.noop_log_id().index();

            let old_progress = self.leader.progress.clone();

            let id_gen = self.state.progress_id_gen.clone();
            let default_entry = |id| {
                let progress_id = StreamId::new(id_gen.next_id());
                ProgressEntry::empty(id, progress_id, end).with_probe_start(probe_start)
            };

            self.leader.progress = Valid::new(old_progress.into_inner().upgrade_quorum_set(
                write_quorum_set.clone(),
                non_write_voter_ids.clone(),
                default_entry,
            ));
        }

        {
            let old_progress = self.leader.clock_progress.clone();

            self.leader.clock_progress = Valid::new(old_progress.into_inner().upgrade_quorum_set(
                write_quorum_set,
                non_write_voter_ids,
                IdVal::new_default,
            ));
            self.leader.update_self_quorum();
        }
    }

    /// Update progress when replicated data(logs or snapshot) matches on follower/learner and is
    /// accepted.
    pub(crate) fn try_update_leader_clock(
        &mut self,
        stream_id: StreamId,
        target: C::NodeId,
        sending_time: SendStamp<InstantOf<C>>,
    ) {
        // clock_progress and progress has the same structure but clock_progress does not store stream_id.
        // Thus we need to check stream_id in progress to ensure the stream is correct.

        tracing::debug!("{}: target: {}, t: {}", func_name!(), &target, sending_time);

        if !self.leader.is_replication_stream_valid(&target, stream_id) {
            return;
        }

        let granted = self.leader.update_clock(&target, sending_time);

        tracing::debug!(
            "granted leader vote clock after updating: granted: {}; clock_progress: {}",
            granted.display(),
            self.leader.clock_progress.display_with(|f, item| write!(f, "{}: {}", item.id, item.val.display()))
        );

        // When membership changes, the granted value may revert to a previous value.
        // E.g.: when membership changes from 12345 to {12345,123}:
        // ```
        // Voters: 1 2 3 4 5
        // Value:  1 1 2 2 2 // 2 is granted by a quorum
        //
        // Voters: 1 2 3 4 5
        //         1 2 3
        // Value:  1 1 2 2 2 // 1 is granted by a quorum
        // ```
    }

    /// Update progress when replicated data(logs or snapshot) matches on follower/learner and is
    /// accepted.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_matching(
        &mut self,
        node_id: C::NodeId,
        log_id: Option<LogIdOf<C>>,
        inflight_id: Option<InflightId>,
    ) {
        tracing::debug!("{}: node_id: {}, log_id: {}", func_name!(), node_id, log_id.display());

        // The value granted by a quorum may not yet be a committed.
        // A committed is **granted** and also is in the current term.
        let Some(quorum_accepted) = self.leader.progress.update_entry_with(&node_id, |entry| {
            entry.new_updater(&*self.config).update_matching(log_id, inflight_id)
        }) else {
            // the node does not exist anymore
            return;
        };

        let quorum_accepted = quorum_accepted.clone();

        tracing::debug!(
            "after updating progress: quorum_accepted: {}",
            quorum_accepted.display()
        );

        self.try_commit_quorum_accepted(quorum_accepted);
    }

    /// Commit the log id that is granted(accepted) by a quorum of voters.
    ///
    /// In raft a log that is granted and in the leader term is committed.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn try_commit_quorum_accepted(&mut self, granted: Option<LogIdOf<C>>) {
        // Only when the log id is proposed by the current leader, it is committed.
        if let Some(ref c) = granted
            && !self.state.vote_ref().is_same_leader(c.committed_leader_id())
        {
            return;
        }

        let prev_cluster_committed = self.state.cluster_committed().cloned();

        let committed = LogIOId::new(self.state.vote_ref().to_committed(), granted.clone());
        self.state.io_state_mut().cluster_committed.try_update(committed).ok();

        // Advance the local apply ceiling regardless; it is a no-op when nothing changed.
        self.state.update_local_committed(&granted);

        // Notify replication only when the cluster-committed log id advanced. The vote component of
        // `cluster_committed` can bump on its own at leadership start with no new committed log id,
        // which must not trigger a broadcast.
        if self.state.cluster_committed() > prev_cluster_committed.as_ref() {
            self.output.push_command(Command::ReplicateCommitted {
                committed: self.state.follower_apply_bound(self.config.enable_relaxed_read),
            });
        }
    }

    /// Update progress when a follower or learner rejects replicated data because its log does not
    /// match the leader's at `conflict`.
    ///
    /// `inflight_id` names the tracked replication operation that the response answers; it does not
    /// tell whether the RPC carried entries. With `Some`, the conflict ends that operation, and the
    /// inflight state becomes `None`; the ID of an operation no longer in flight leaves the
    /// progress entry unchanged. With `None`, as for a heartbeat's conflict, no operation ends, but
    /// when log reversion is allowed and `conflict` is below `matching`, resetting `matching` also
    /// drops a `LogsSince` plan.
    ///
    /// Returns `false` if the response is stale or `target` has no progress entry, so nothing was
    /// applied.
    ///
    /// A conflict that drops the target's `LogsSince` plan starts an [`Inflight::Clear`], because
    /// the stream that runs the plan may still read the log: a heartbeat's conflict, such as one
    /// that finds the target's log reverted, never reaches the stream, and a stream pipelines its
    /// reads ahead of the answers. The logs the plan could read stay unpurged until the replication
    /// task confirms the clear, see [`Self::update_cleared()`].
    ///
    /// This method intentionally bypasses `VecProgress::update_entry_with()`: when log
    /// reversion is explicitly allowed, [`Updater::update_conflicting()`] may reset
    /// `matching` to `None`. `VecProgress::reset_entry_with()` moves the entry down to
    /// keep the progress ordering, but deliberately does not lower the recorded
    /// quorum-accepted value: a value accepted by a quorum must never be withdrawn.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_conflicting(
        &mut self,
        target: C::NodeId,
        conflict: LogIdOf<C>,
        inflight_id: Option<InflightId>,
    ) -> bool {
        let mut applied = false;
        let mut dropped = None;

        self.leader.progress.reset_entry_with(&target, |entry| {
            let plan = entry.data.inflight.clone();
            let mut updater = Updater::new(&*self.config, entry);
            applied = updater.update_conflicting(conflict.index(), inflight_id);
            if plan.is_logs_since() && entry.data.inflight.is_none() {
                dropped = Some(plan);
            }
        });

        if let Some(plan) = dropped {
            self.clear_plan(target, plan);
        }

        applied
    }

    /// Narrow the search range of `target` with its conflict hint, and choose the next probe.
    fn update_search_from_hint(&mut self, target: &C::NodeId, hint: ConflictHint<C>) {
        let Some(progress) = self.leader.progress.try_get(target) else {
            return;
        };

        let matching_next = progress.matching().next_index();

        let (common, divergence) = hint.compare(&self.state.log_ids);

        // A hint can only narrow the search range, and never below `matching`.
        let end = std::cmp::min(divergence, progress.data.searching_end);
        let end = std::cmp::max(end, matching_next);

        // Probe right after the greatest shared log ID. An empty hint means an empty follower log.
        let probe = match common {
            Some(log_id) => Some(log_id.index() + 1),
            None if hint.log_ids.is_empty() => Some(0),
            None => None,
        };
        let probe = probe.filter(|index| matching_next <= *index && *index <= end);

        self.leader.progress.update_data_with(target, |data| {
            // Only the probe's durable response advances matching.
            data.searching_end = end;
            data.probe_start = probe;
        });
    }

    /// Set whether the log of `target` may revert once.
    ///
    /// Sets the one-shot `allow_log_reversion` permission of `target` to `allow`. While it is set,
    /// a conflict below the target's `matching`, as from a follower that lost its log, resets
    /// `matching` to `None` and restarts the search for the matching log; that reset clears the
    /// permission. Passing `false` clears an unused permission. [`Config::allow_log_reversion`]
    /// allows every reversion, whatever this permission is.
    ///
    /// Returns [`NodeNotFound`] if `target` has no progress entry.
    ///
    /// [`Config::allow_log_reversion`]: crate::config::Config::allow_log_reversion
    pub(crate) fn allow_next_revert(&mut self, target: C::NodeId, allow: bool) -> Result<(), NodeNotFound<C::NodeId>> {
        let res = self.leader.progress.update_data_with(&target, |data| data.allow_log_reversion = allow);

        if res.is_none() {
            tracing::warn!(
                "target node {} not found in progress tracker, when {}",
                target,
                func_name!()
            );
            return Err(NodeNotFound::new(target, Operation::AllowNextRevert));
        }

        Ok(())
    }

    /// Update replication progress when a response is received.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_progress(
        &mut self,
        stream_id: StreamId,
        progress: Progress<C>,
        inflight_id: Option<InflightId>,
    ) {
        let Progress {
            target,
            result: repl_res,
            hint,
        } = progress;

        tracing::debug!(
            "{}: target: {}, stream_id: {}, result: {}, inflight_id: {}, current progresses: {}",
            func_name!(),
            target,
            stream_id,
            repl_res.display(),
            inflight_id.display(),
            self.leader.progress
        );

        if !self.leader.is_replication_stream_valid(&target, stream_id) {
            return;
        }

        match repl_res {
            Ok(p) => match p.0 {
                Ok(matching) => {
                    self.update_matching(target, matching, inflight_id);
                }
                Err(conflict) => {
                    let applied = self.update_conflicting(target.clone(), conflict, inflight_id);

                    // Apply the hint only if the conflict answered the current request. A conflict
                    // without an inflight id, such as a heartbeat's, cannot be tied to it.
                    if applied
                        && inflight_id.is_some()
                        && let Some(hint) = hint
                    {
                        self.update_search_from_hint(&target, hint);
                    }
                }
            },
            Err(err_str) => {
                tracing::warn!("update progress error: {}", err_str);

                // Reset the operation the error belongs to, and it will retry. An error without an
                // operation ID, or with the ID of an earlier operation, leaves the current one: a
                // stream can report an error after an earlier answer completed its operation and
                // another one started. A clear stays: it waits for the replication task, which may
                // still read the log for the plan the clear dropped, and an error of that plan says
                // nothing about whether it still does.
                let mut dropped = None;
                self.leader.progress.update_data_with(&target, |data| {
                    if data.inflight.inflight_id() != inflight_id {
                        return;
                    }
                    match data.inflight {
                        Inflight::Probe { .. } | Inflight::LogsSince { .. } => {
                            dropped = Some(std::mem::replace(&mut data.inflight, Inflight::None));
                        }
                        Inflight::Snapshot { .. } => data.inflight = Inflight::None,
                        Inflight::None | Inflight::Clear { .. } => {}
                    }
                });

                // A transport that pulls requests on a task of its own may still read the log for
                // the dropped plan.
                if let Some(plan) = dropped {
                    self.clear_plan(target, plan);
                }
            }
        };

        // The purge job may be postponed because a replication task is using them.
        // Thus, we just try again to purge when progress is updated.
        self.try_purge_log();
    }

    /// Drop `plan`, the log plan of `target`, and ask the replication task to confirm that it no
    /// longer reads the log for it.
    ///
    /// Until the task confirms, the progress entry holds back the purge of the logs the plan could
    /// read and starts no other plan, see [`Inflight::Clear`].
    fn clear_plan(&mut self, target: C::NodeId, plan: Inflight<C>) {
        let inflight_id = self.state.new_inflight_id();
        let Some(clear) = plan.into_clear(inflight_id) else {
            return;
        };

        self.leader.progress.update_data_with(&target, |data| data.inflight = clear);
        self.output.push_command(Command::ClearReplicate { target, inflight_id });
    }

    /// Apply the replication task's answer to [`Command::ClearReplicate`]: it no longer reads the
    /// log for the plan the leader dropped, so the purge that the plan held back may run.
    ///
    /// An answer to an earlier clear, or from a replication stream that has been replaced, is
    /// ignored.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_cleared(&mut self, stream_id: StreamId, target: C::NodeId, inflight_id: InflightId) {
        tracing::debug!(
            "{}: target: {}, stream_id: {}, inflight_id: {}",
            func_name!(),
            target,
            stream_id,
            inflight_id
        );

        if !self.leader.is_replication_stream_valid(&target, stream_id) {
            return;
        }

        let mut cleared = false;
        self.leader.progress.update_data_with(&target, |data| cleared = data.inflight.cleared(inflight_id));

        if cleared {
            self.try_purge_log();
        }
    }

    /// Update replication streams to reflect replication progress change.
    ///
    /// With `close_old`, `RaftCore` replaces every replication stream, so the work in flight of
    /// every target is dropped. Without it, `RaftCore` keeps the stream of each target that still
    /// receives logs, and closes the stream of a target that no longer does. The work in flight of
    /// such a target is dropped too: the closed stream never answers it, and it would keep the
    /// target from starting new work once the target receives logs again and gets a new stream.
    /// The target keeps its matching log id and its heartbeats.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn rebuild_replication_streams(&mut self, close_old: bool) {
        let mut targets = vec![];

        let membership = self.state.membership_state.effective();

        for item in self.leader.progress.iter_mut_without_reorder() {
            if item.id != self.config.id {
                if close_old || !membership.membership().receives_logs(&item.id) {
                    item.data.inflight = Inflight::None;
                }

                let target_node = membership.get_node(&item.id).unwrap().clone();

                targets.push(TargetProgress {
                    target_node,
                    progress: item.clone(),
                });
            }
        }
        self.output.push_command(Command::RebuildReplicationStreams {
            leader_vote: self.leader.committed_vote.clone(),
            targets,
            close_old_streams: close_old,
        });
    }

    /// Initiate replication for every target that is not sending data in flight.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn initiate_replication(&mut self) {
        tracing::debug!("{}: progress: {:?}", func_name!(), self.leader.progress);

        let membership = self.state.membership_state.effective().clone();
        for item in self.leader.progress.iter_mut_without_reorder() {
            // The leader's own matching is updated by `update_local_progress()`.
            if item.id == self.config.id {
                continue;
            }
            let receives_logs = membership.membership().receives_logs(&item.id);
            if !receives_logs {
                continue;
            }

            let target = item.id.clone();
            let t = item.next_send(self.state, MAX_ENTRIES_PER_PROBE);
            tracing::debug!("next send: target: {}, send: {:?}", target, t);

            match t {
                Ok(inflight) => {
                    let leader_vote = self.leader.committed_vote.clone();
                    Self::send_to_target(self.output, leader_vote, &target, inflight);
                }
                Err(e) => {
                    tracing::debug!("no data to replicate for node-{}: current inflight: {:?}", target, e,);
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_to_target(
        output: &mut EngineOutput<C, SM>,
        leader_vote: CommittedVoteOf<C>,
        target: &C::NodeId,
        inflight: &Inflight<C>,
    ) {
        match inflight {
            Inflight::None => unreachable!("no data to send"),
            Inflight::Clear { .. } => unreachable!("next_send() never plans a clear"),
            Inflight::Probe {
                log_id_range,
                inflight_id,
            } => {
                let req = Replicate::new_probe(log_id_range.clone(), *inflight_id);
                output.push_command(Command::Replicate {
                    target: target.clone(),
                    req,
                });
            }
            Inflight::Snapshot { inflight_id } => {
                output.push_command(Command::ReplicateSnapshot {
                    leader_vote,
                    target: target.clone(),
                    inflight_id: *inflight_id,
                });
            }
            Inflight::LogsSince { prev, inflight_id } => {
                let req = Replicate::new_logs_since(prev.clone(), *inflight_id);
                output.push_command(Command::Replicate {
                    target: target.clone(),
                    req,
                });
            }
        };
    }

    /// Try to run a pending purge job if no tasks are using the logs to be purged.
    ///
    /// Purging logs involves concurrent log accesses by replication tasks and purging tasks.
    /// Therefore, it is a method of ReplicationHandler.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn try_purge_log(&mut self) {
        tracing::debug!(
            "try_purge_log: last_purged_log_id: {}, purge_upto: {}",
            self.state.last_purged_log_id().display(),
            self.state.purge_upto().display()
        );

        if self.state.purge_upto() <= self.state.last_purged_log_id() {
            tracing::debug!("no need to purge, return");
            return;
        }

        // Safe unwrap(): it greater than an Option thus it must be a Some()
        let purge_upto = self.state.purge_upto().unwrap().clone();

        // Check if any replication task is going to use the log that is going to purge.
        let mut in_use = false;
        for item in self.leader.progress.iter() {
            if item.is_log_range_inflight(&purge_upto) {
                tracing::debug!("log {} is in use by {}", purge_upto, item.id);
                in_use = true;
            }
        }

        if in_use {
            // Logs to purge is in use, postpone purging.
            tracing::debug!("cannot purge: {} is in use", purge_upto);
            return;
        }

        self.log_handler().purge_log();
    }

    /// Update the progress of local log to `upto`(inclusive).
    ///
    /// Writing to local log store does not have to wait for a replication response from remote
    /// nodes. Thus, it can just be done in a fast-path.
    pub(crate) fn update_local_progress(&mut self, upto: Option<LogIdOf<C>>) {
        tracing::debug!("{}: upto: {}", func_name!(), upto.display());

        if upto.is_none() {
            return;
        }

        let id = self.config.id.clone();

        // The leader may not be in membership anymore
        if let Some(entry) = self.leader.progress.try_get(&id) {
            tracing::debug!("update progress: self_matching: {}", entry.matching().display());

            if entry.matching() >= upto.as_ref() {
                return;
            }
        }

        let inflight_id = InflightId::new(0);

        if self
            .leader
            .progress
            .update_data_with(&id, |data| {
                // `None` as `prev` is fine here: this records the local log's own progress.
                data.inflight = Inflight::probe(None, upto.clone(), inflight_id);
            })
            .is_some()
        {
            self.update_matching(id, upto, Some(inflight_id));
        }
    }

    pub(crate) fn log_handler(&mut self) -> LogHandler<'_, C, SM> {
        LogHandler::new(self.config, self.state, self.output)
    }
}
