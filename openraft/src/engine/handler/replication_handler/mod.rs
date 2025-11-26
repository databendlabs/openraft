use crate::EffectiveMembership;
use crate::LogIdOptionExt;
use crate::Membership;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::ServerState;
use crate::display_ext::DisplayInstantExt;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplayResultExt;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::engine::ReplicationProgress;
use crate::engine::handler::log_handler::LogHandler;
use crate::error::NodeNotFound;
use crate::error::Operation;
use crate::progress;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::proposer::Leader;
use crate::proposer::LeaderQuorumSet;
use crate::raft_state::LogStateReader;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::replication::request::Replicate;
use crate::replication::response::ReplicationResult;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
use crate::vote::raft_vote::RaftVoteExt;

#[cfg(test)]
mod append_membership_test;
#[cfg(test)]
mod update_matching_test;

/// Handle replication operations.
///
/// - Writing local log store;
/// - Replicating log to a remote node;
/// - Tracking membership changes and update related state;
/// - Tracking replication progress and commit;
/// - Purging in-snapshot logs;
/// - etc.
pub(crate) struct ReplicationHandler<'x, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'x mut EngineConfig<C>,
    pub(crate) leader: &'x mut Leader<C, LeaderQuorumSet<C>>,
    pub(crate) state: &'x mut RaftState<C>,
    pub(crate) output: &'x mut EngineOutput<C>,
}

impl<C> ReplicationHandler<'_, C>
where C: RaftTypeConfig
{
    /// Append a new membership and update related state such as replication streams.
    ///
    /// It is called by the leader when a new membership log is appended to the log store.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn append_membership(&mut self, log_id: &LogIdOf<C>, m: &Membership<C>) {
        tracing::debug!("update effective membership: log_id:{} {}", log_id, m);

        debug_assert!(
            self.state.server_state == ServerState::Leader,
            "Only leader is allowed to call update_effective_membership()"
        );
        debug_assert!(
            self.state.is_leader(&self.config.id),
            "Only leader is allowed to call update_effective_membership()"
        );

        self.state.membership_state.append(EffectiveMembership::new_arc(Some(log_id.clone()), m.clone()));

        // TODO(9): currently only a leader has replication setup.
        //       It's better to setup replication for both leader and candidate.
        //       e.g.: if self.internal_server_state.is_leading() {

        // Leader does not quit at once:
        // A leader should always keep replicating logs.
        // A leader that is removed will shut down replications when this membership log is
        // committed.

        self.rebuild_progresses();
        self.rebuild_replication_streams();
        self.initiate_replication();
    }

    /// Rebuild leader's replication progress to reflect replication changes.
    ///
    /// E.g., when adding/removing a follower/learner.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn rebuild_progresses(&mut self) {
        let em = self.state.membership_state.effective();

        let learner_ids = em.learner_ids().collect::<Vec<_>>();

        {
            let end = self.state.last_log_id().next_index();
            let default_v = || ProgressEntry::empty(end);

            let old_progress = self.leader.progress.clone();

            self.leader.progress =
                old_progress.upgrade_quorum_set(em.membership().to_quorum_set(), learner_ids.clone(), default_v);
        }

        {
            let old_progress = self.leader.clock_progress.clone();

            self.leader.clock_progress =
                old_progress.upgrade_quorum_set(em.membership().to_quorum_set(), learner_ids, || None);
        }
    }

    /// Update progress when replicated data(logs or snapshot) matches on follower/learner and is
    /// accepted.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_leader_clock(&mut self, node_id: C::NodeId, t: InstantOf<C>) {
        tracing::debug!(target = display(&node_id), t = display(t.display()), "{}", func_name!());

        let granted = *self
            .leader
            .clock_progress
            .increase_to(&node_id, Some(t))
            .expect("it should always update existing progress");

        tracing::debug!(
            granted = display(granted.as_ref().map(|x| x.display()).display()),
            clock_progress = display(
                &self
                    .leader
                    .clock_progress
                    .display_with(|f, id, v| { write!(f, "{}: {}", id, v.as_ref().map(|x| x.display()).display()) })
            ),
            "granted leader vote clock after updating"
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
        tracing::debug!(
            node_id = display(&node_id),
            log_id = display(log_id.display()),
            "{}",
            func_name!()
        );

        debug_assert!(log_id.is_some(), "a valid update can never set matching to None");

        // The value granted by a quorum may not yet be a committed.
        // A committed is **granted** and also is in the current term.
        let quorum_accepted = self
            .leader
            .progress
            .update_with(&node_id, |prog_entry| {
                prog_entry.new_updater(&*self.config).update_matching(log_id, inflight_id)
            })
            .expect("it should always update existing progress")
            .clone();

        tracing::debug!(
            quorum_accepted = display(quorum_accepted.display()),
            "after updating progress"
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

        let committed = LogIOId::new(self.state.vote_ref().to_committed(), granted.clone());
        self.state.io_state_mut().cluster_committed.try_update(committed.clone()).ok();

        if let Some(_prev_committed) = self.state.update_local_committed(&granted) {
            self.output.push_command(Command::ReplicateCommitted {
                committed: self.state.committed().cloned(),
            });
        }
    }

    /// Update progress when replicated data(logs or snapshot) does not match the follower/learner
    /// state and is rejected.
    ///
    /// If `has_payload` is true, the `inflight` state is reset because AppendEntries RPC
    /// manages the inflight state.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_conflicting(
        &mut self,
        target: C::NodeId,
        conflict: LogIdOf<C>,
        inflight_id: Option<InflightId>,
    ) {
        // TODO(2): test it?

        let prog_entry = self.leader.progress.get_mut(&target).unwrap();

        let mut updater = progress::entry::update::Updater::new(self.config, prog_entry);

        updater.update_conflicting(conflict.index(), inflight_id);
    }

    /// Enable one-time replication reset for a specific node upon log reversion detection.
    ///
    /// This method sets a flag to allow the replication process to be reset once for the specified
    /// target node when a log reversion is detected. This is typically used to handle scenarios
    /// where a follower node's log has unexpectedly reverted to a previous state.
    ///
    /// # Behavior
    ///
    /// - Sets the `reset_on_reversion` flag to `true` for the specified node in the leader's
    ///   progress tracker.
    /// - This flag will be consumed upon the next log reversion detection, allowing for a one-time
    ///   reset.
    /// - If the node is not found in the progress tracker, this method ignore it.
    pub(crate) fn allow_next_revert(&mut self, target: C::NodeId, allow: bool) -> Result<(), NodeNotFound<C>> {
        let Some(prog_entry) = self.leader.progress.get_mut(&target) else {
            tracing::warn!(
                "target node {} not found in progress tracker, when {}",
                target,
                func_name!()
            );
            return Err(NodeNotFound::new(target, Operation::AllowNextRevert));
        };

        prog_entry.allow_log_reversion = allow;

        Ok(())
    }

    /// Update replication progress when a response is received.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_progress(
        &mut self,
        target: C::NodeId,
        repl_res: Result<ReplicationResult<C>, String>,
        inflight_id: Option<InflightId>,
    ) {
        tracing::debug!(
            "{}: target={target}, result={}, inflight_id={}, current progresses={}",
            func_name!(),
            repl_res.display(),
            inflight_id.display(),
            self.leader.progress
        );

        match repl_res {
            Ok(p) => match p.0 {
                Ok(matching) => {
                    self.update_matching(target, matching, inflight_id);
                }
                Err(conflict) => {
                    self.update_conflicting(target, conflict, inflight_id);
                }
            },
            Err(err_str) => {
                tracing::warn!(result = display(&err_str), "update progress error");

                // Reset inflight state and it will retry.
                let p = self.leader.progress.get_mut(&target).unwrap();
                p.inflight = Inflight::None;
            }
        };

        // The purge job may be postponed because a replication task is using them.
        // Thus, we just try again to purge when progress is updated.
        self.try_purge_log();
    }

    /// Update replication streams to reflect replication progress change.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn rebuild_replication_streams(&mut self) {
        let mut targets = vec![];

        // TODO: maybe it's better to update leader's matching when update_replication() is called.
        for item in self.leader.progress.iter_mut() {
            if item.id != self.config.id {
                // Reset and resend (by self.send_to_all()) replication requests.
                item.val.inflight = Inflight::None;

                targets.push(ReplicationProgress(item.id.clone(), item.val.clone()));
            }
        }
        self.output.push_command(Command::RebuildReplicationStreams { targets });
    }

    /// Initiate replication for every target that is not sending data in flight.
    ///
    /// `send_none` specifies whether to force to send a message even when there is no data to send.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn initiate_replication(&mut self) {
        tracing::debug!(progress = debug(&self.leader.progress), "{}", func_name!());

        for item in self.leader.progress.iter_mut() {
            // TODO: update matching should be done here for leader
            //       or updating matching should be queued in commands?
            if item.id == self.config.id {
                continue;
            }

            let t = item.val.next_send(self.state, self.config.max_payload_entries);
            tracing::debug!(target = display(&item.id), send = debug(&t), "next send");

            match t {
                Ok(inflight) => {
                    Self::send_to_target(self.output, &item.id, inflight);
                }
                Err(e) => {
                    tracing::debug!("no data to replicate for node-{}: current inflight: {:?}", item.id, e,);
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_to_target(output: &mut EngineOutput<C>, target: &C::NodeId, inflight: &Inflight<C>) {
        match inflight {
            Inflight::None => unreachable!("no data to send"),
            Inflight::Logs {
                log_id_range,
                inflight_id,
            } => {
                let req = Replicate::logs(log_id_range.clone(), *inflight_id);
                output.push_command(Command::Replicate {
                    target: target.clone(),
                    req,
                });
            }
            Inflight::Snapshot { inflight_id } => {
                output.push_command(Command::ReplicateSnapshot {
                    target: target.clone(),
                    inflight_id: *inflight_id,
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
        // TODO refactor this
        // TODO: test

        tracing::debug!(
            last_purged_log_id = display(self.state.last_purged_log_id().display()),
            purge_upto = display(self.state.purge_upto().display()),
            "try_purge_log"
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
            if item.val.is_log_range_inflight(&purge_upto) {
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

    // TODO: replication handler should provide the same API for both locally and remotely log
    // writing.       This may simplify upper level accessing.
    /// Update the progress of local log to `upto`(inclusive).
    ///
    /// Writing to local log store does not have to wait for a replication response from remote
    /// nodes. Thus, it can just be done in a fast-path.
    pub(crate) fn update_local_progress(&mut self, upto: Option<LogIdOf<C>>) {
        tracing::debug!(upto = display(upto.display()), "{}", func_name!());

        if upto.is_none() {
            return;
        }

        let id = self.config.id.clone();

        // The leader may not be in membership anymore
        if let Some(prog_entry) = self.leader.progress.get_mut(&id) {
            tracing::debug!(
                self_matching = display(prog_entry.matching().display()),
                "update progress"
            );

            if prog_entry.matching() >= upto.as_ref() {
                return;
            }
            // TODO: It should be self.state.last_log_id() but None is ok.
            prog_entry.inflight = Inflight::logs(None, upto.clone(), InflightId::new(0));

            self.update_matching(id, upto, Some(InflightId::new(0)));
        }
    }

    pub(crate) fn log_handler(&mut self) -> LogHandler<'_, C> {
        LogHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }
}
