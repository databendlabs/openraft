use std::ops::Deref;

use crate::display_ext::DisplayOptionExt;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::handler::snapshot_handler::SnapshotHandler;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::proposer::Leader;
use crate::proposer::LeaderQuorumSet;
use crate::raft_state::LogStateReader;
use crate::replication::request_id::RequestId;
use crate::replication::response::ReplicationResult;
use crate::type_config::alias::InstantOf;
use crate::AsyncRuntime;
use crate::EffectiveMembership;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::Membership;
use crate::MessageSummary;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::ServerState;

#[cfg(test)]
mod append_membership_test;
#[cfg(test)]
mod update_matching_test;

/// Handle replication operations.
///
/// - Writing local log store;
/// - Replicating log to remote node;
/// - Tracking membership changes and update related state;
/// - Tracking replication progress and commit;
/// - Purging in-snapshot logs;
/// - etc
pub(crate) struct ReplicationHandler<'x, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'x mut EngineConfig<C::NodeId>,
    pub(crate) leader: &'x mut Leader<C, LeaderQuorumSet<C::NodeId>>,
    pub(crate) state: &'x mut RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>,
    pub(crate) output: &'x mut EngineOutput<C>,
}

/// An option about whether to send an RPC to follower/learner even when there is no data to send.
///
/// Sending none data serves as a heartbeat.
#[derive(Debug)]
#[derive(PartialEq, Eq)]
pub(crate) enum SendNone {
    False,
    True,
}

impl<C> ReplicationHandler<'_, C>
where C: RaftTypeConfig
{
    /// Append a new membership and update related state such as replication streams.
    ///
    /// It is called by the leader when a new membership log is appended to log store.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn append_membership(&mut self, log_id: &LogId<C::NodeId>, m: &Membership<C::NodeId, C::Node>) {
        tracing::debug!("update effective membership: log_id:{} {}", log_id, m.summary());

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
        self.initiate_replication(SendNone::False);
    }

    /// Rebuild leader's replication progress to reflect replication changes.
    ///
    /// E.g. when adding/removing a follower/learner.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn rebuild_progresses(&mut self) {
        let em = self.state.membership_state.effective();

        let learner_ids = em.learner_ids().collect::<Vec<_>>();

        {
            let end = self.state.last_log_id().next_index();
            let default_v = ProgressEntry::empty(end);

            let old_progress = self.leader.progress.clone();

            self.leader.progress =
                old_progress.upgrade_quorum_set(em.membership().to_quorum_set(), &learner_ids, default_v);
        }

        {
            let old_progress = self.leader.clock_progress.clone();

            self.leader.clock_progress =
                old_progress.upgrade_quorum_set(em.membership().to_quorum_set(), &learner_ids, None);
        }
    }

    /// Update replication progress when a successful response is received.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_success_progress(
        &mut self,
        target: C::NodeId,
        request_id: RequestId,
        result: ReplicationResult<C>,
    ) {
        // No matter what the result is, the validity of the leader is granted by a follower.
        self.update_leader_clock(target.clone(), result.sending_time);

        let id = request_id.request_id();

        // If request id is `None`, it means there is not data in the request payload, thus no need to
        // update the matching pointer.
        // But conflict should always be handled, e.g., especially in scenario the follower state reverted.
        match result.result {
            Ok(matching) => {
                let Some(id) = id else {
                    tracing::debug!(request_id = display(request_id), "no data for this request, return");
                    return;
                };
                self.update_matching(target.clone(), id, matching);
            }
            Err(conflict) => {
                self.update_conflicting(target, id, conflict);
            }
        }
    }

    /// Update progress when replicated data(logs or snapshot) matches on follower/learner and is
    /// accepted.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_leader_clock(&mut self, node_id: C::NodeId, t: InstantOf<C>) {
        tracing::debug!(target = display(&node_id), t = debug(t), "{}", func_name!());

        let granted = *self
            .leader
            .clock_progress
            .increase_to(&node_id, Some(t))
            .expect("it should always update existing progress");

        tracing::debug!(
            granted = debug(granted),
            clock_progress = debug(&self.leader.clock_progress),
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
    pub(crate) fn update_matching(&mut self, node_id: C::NodeId, inflight_id: u64, log_id: Option<LogId<C::NodeId>>) {
        tracing::debug!(
            node_id = display(&node_id),
            inflight_id = display(inflight_id),
            log_id = display(log_id.display()),
            "{}",
            func_name!()
        );

        debug_assert!(log_id.is_some(), "a valid update can never set matching to None");

        // The value granted by a quorum may not yet be a committed.
        // A committed is **granted** and also is in current term.
        let quorum_accepted = self
            .leader
            .progress
            .update_with(&node_id, |prog_entry| {
                let res = prog_entry.update_matching(inflight_id, log_id);
                if let Err(e) = &res {
                    tracing::error!(error = display(e), "update_matching");
                    panic!("update_matching error: {}", e);
                }
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
    pub(crate) fn try_commit_quorum_accepted(&mut self, granted: Option<LogId<C::NodeId>>) {
        // Only when the log id is proposed by current leader, it is committed.
        if let Some(c) = granted.clone() {
            if !self.state.vote_ref().is_same_leader(c.committed_leader_id()) {
                return;
            }
        }

        if let Some(prev_committed) = self.state.update_committed(&granted) {
            self.output.push_command(Command::ReplicateCommitted {
                committed: self.state.committed().cloned(),
            });

            let seq = self.output.next_sm_seq();
            self.output.push_command(Command::Commit {
                seq,
                already_committed: prev_committed,
                upto: self.state.committed().cloned().unwrap(),
            });

            if self.config.snapshot_policy.should_snapshot(&self.state) {
                self.snapshot_handler().trigger_snapshot();
            }
        }
    }

    /// Update progress when replicated data(logs or snapshot) does not match follower/learner state
    /// and is rejected.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_conflicting(
        &mut self,
        target: C::NodeId,
        inflight_id: Option<u64>,
        conflict: LogId<C::NodeId>,
    ) {
        // TODO(2): test it?

        let prog_entry = self.leader.progress.get_mut(&target).unwrap();

        prog_entry.update_conflicting(inflight_id, conflict.index).unwrap();
    }

    /// Update replication progress when a response is received.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_progress(
        &mut self,
        target: C::NodeId,
        request_id: RequestId,
        repl_res: Result<ReplicationResult<C>, String>,
    ) {
        tracing::debug!(
            target = display(&target),
            request_id = display(request_id),
            result = debug(&repl_res),
            progress = display(&self.leader.progress),
            "{}",
            func_name!()
        );

        match repl_res {
            Ok(p) => {
                self.update_success_progress(target.clone(), request_id, p);
            }
            Err(err_str) => {
                tracing::warn!(
                    request_id = display(request_id),
                    result = display(&err_str),
                    "update progress error"
                );

                if request_id == RequestId::HeartBeat {
                    tracing::warn!("heartbeat error: {}, no update to inflight data", err_str);
                } else {
                    // Reset inflight state and it will retry.
                    let p = self.leader.progress.get_mut(&target).unwrap();

                    debug_assert!(
                        p.inflight.is_my_id(request_id),
                        "inflight({:?}) id should match: {}",
                        p.inflight,
                        request_id
                    );

                    p.inflight = Inflight::None;
                }
            }
        };

        // The purge job may be postponed because a replication task is using them.
        // Thus we just try again to purge when progress is updated.
        self.try_purge_log();

        // initialize next replication to this target

        {
            let p = self.leader.progress.get_mut(&target).unwrap();

            let r = p.next_send(self.state.deref(), self.config.max_payload_entries);
            tracing::debug!(next_send_res = debug(&r), "next_send");

            if let Ok(inflight) = r {
                Self::send_to_target(self.output, &target, inflight);
            } else {
                tracing::debug!("nothing to send to target={target}, progress:{}", p);
            }
        }
    }

    /// Update replication streams to reflect replication progress change.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn rebuild_replication_streams(&mut self) {
        let mut targets = vec![];

        // TODO: maybe it's better to update leader's matching when update_repliation() is called.
        for (target, prog_entry) in self.leader.progress.iter_mut() {
            if target != &self.config.id {
                // Reset and resend(by self.send_to_all()) replication requests.
                prog_entry.inflight = Inflight::None;

                targets.push((target.clone(), prog_entry.clone()));
            }
        }
        self.output.push_command(Command::RebuildReplicationStreams { targets });
    }

    /// Initiate replication for every target that is not sending data in flight.
    ///
    /// `send_none` specifies whether to force to send a message even when there is no data to send.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn initiate_replication(&mut self, send_none: SendNone) {
        tracing::debug!(progress = debug(&self.leader.progress), "{}", func_name!());

        for (id, prog_entry) in self.leader.progress.iter_mut() {
            // TODO: update matching should be done here for leader
            //       or updating matching should be queued in commands?
            if id == &self.config.id {
                continue;
            }

            let t = prog_entry.next_send(self.state, self.config.max_payload_entries);
            tracing::debug!(target = display(&*id), send = debug(&t), "next send");

            match t {
                Ok(inflight) => {
                    Self::send_to_target(self.output, id, inflight);
                }
                Err(e) => {
                    tracing::debug!(
                        "no data to replicate for node-{}: current inflight: {:?}, send_none: {:?}",
                        id,
                        e,
                        send_none
                    );

                    #[allow(clippy::collapsible_if)]
                    if e == &Inflight::None {
                        if send_none == SendNone::True {
                            Self::send_to_target(self.output, id, e);
                        }
                    }
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_to_target(output: &mut EngineOutput<C>, target: &C::NodeId, inflight: &Inflight<C::NodeId>) {
        output.push_command(Command::Replicate {
            target: target.clone(),
            req: inflight.clone(),
        });
    }

    /// Try to run a pending purge job, if no tasks are using the logs to be purged.
    ///
    /// Purging logs involves concurrent log accesses by replication tasks and purging task.
    /// Therefor it is a method of ReplicationHandler.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn try_purge_log(&mut self) {
        // TODO refactor this
        // TODO: test

        tracing::debug!(
            last_purged_log_id = display(self.state.last_purged_log_id().summary()),
            purge_upto = display(self.state.purge_upto().summary()),
            "try_purge_log"
        );

        if self.state.purge_upto() <= self.state.last_purged_log_id() {
            tracing::debug!("no need to purge, return");
            return;
        }

        // Safe unwrap(): it greater than an Option thus it must be a Some()
        let purge_upto = self.state.purge_upto().unwrap().clone();

        // Check if any replication task is going to use the log that are going to purge.
        let mut in_use = false;
        for (id, prog_entry) in self.leader.progress.iter() {
            if prog_entry.is_log_range_inflight(&purge_upto) {
                tracing::debug!("log {} is in use by {}", purge_upto, id);
                in_use = true;
            }
        }

        if in_use {
            // Logs to purge is in use, postpone purging.
            tracing::debug!("can not purge: {} is in use", purge_upto);
            return;
        }

        self.log_handler().purge_log();
    }

    // TODO: replication handler should provide the same API for both locally and remotely log
    // writing.       This may simplify upper level accessing.
    /// Update the progress of local log to `upto`(inclusive).
    ///
    /// Writing to local log store does not have to wait for a replication response from remote
    /// node. Thus it can just be done in a fast-path.
    pub(crate) fn update_local_progress(&mut self, upto: Option<LogId<C::NodeId>>) {
        tracing::debug!(upto = display(upto.display()), "{}", func_name!());

        if upto.is_none() {
            return;
        }

        let id = self.config.id.clone();

        // The leader may not be in membership anymore
        if let Some(prog_entry) = self.leader.progress.get_mut(&id) {
            tracing::debug!(
                self_matching = display(prog_entry.matching.display()),
                "update progress"
            );

            if prog_entry.matching >= upto {
                return;
            }
            // TODO: It should be self.state.last_log_id() but None is ok.
            prog_entry.inflight = Inflight::logs(None, upto.clone());

            let inflight_id = prog_entry.inflight.get_id().unwrap();
            self.update_matching(id, inflight_id, upto);
        }
    }

    pub(crate) fn log_handler(&mut self) -> LogHandler<C> {
        LogHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }

    fn snapshot_handler(&mut self) -> SnapshotHandler<C> {
        SnapshotHandler {
            state: self.state,
            output: self.output,
        }
    }
}
