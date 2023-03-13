use std::ops::Deref;

use crate::engine::engine_impl::EngineOutput;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::internal_server_state::LeaderQuorumSet;
use crate::leader::Leader;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::raft_state::LogStateReader;
use crate::replication::ReplicationResult;
use crate::EffectiveMembership;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::Membership;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RaftState;
use crate::ServerState;

#[cfg(test)] mod append_membership_test;
#[cfg(test)] mod update_matching_test;

/// Handle replication operations.
///
/// - Writing local log store;
/// - Replicating log to remote node;
/// - Tracking membership changes and update related state;
/// - Tracking replication progress and commit;
/// - Purging in-snapshot logs;
/// - etc
pub(crate) struct ReplicationHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) config: &'x mut EngineConfig<NID>,
    pub(crate) leader: &'x mut Leader<NID, LeaderQuorumSet<NID>>,
    pub(crate) state: &'x mut RaftState<NID, N>,
    pub(crate) output: &'x mut EngineOutput<NID, N>,
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

impl<'x, NID, N> ReplicationHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Append a blank log.
    ///
    /// It is used by the leader when leadership is established.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn append_blank_log(&mut self) {
        let log_id = LogId::new(
            self.state.vote_ref().committed_leader_id().unwrap(),
            self.state.last_log_id().next_index(),
        );
        self.state.log_ids.append(log_id);
        self.output.push_command(Command::AppendBlankLog { log_id });

        self.update_local_progress(Some(log_id));
    }

    /// Append a new membership and update related state such as replication streams.
    ///
    /// It is called by the leader when a new membership log is appended to log store.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn append_membership(&mut self, log_id: &LogId<NID>, m: &Membership<NID, N>) {
        tracing::debug!("update effective membership: log_id:{} {}", log_id, m.summary());

        debug_assert!(
            self.state.server_state == ServerState::Leader,
            "Only leader is allowed to call update_effective_membership()"
        );
        debug_assert!(
            self.state.is_leader(&self.config.id),
            "Only leader is allowed to call update_effective_membership()"
        );

        self.state.membership_state.append(EffectiveMembership::new_arc(Some(*log_id), m.clone()));

        let em = self.state.membership_state.effective();
        self.output.push_command(Command::UpdateMembership { membership: em.clone() });

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

        let end = self.state.last_log_id().next_index();

        let old_progress = self.leader.progress.clone();
        let learner_ids = em.learner_ids().collect::<Vec<_>>();

        self.leader.progress =
            old_progress.upgrade_quorum_set(em.membership().to_quorum_set(), &learner_ids, ProgressEntry::empty(end));
    }

    /// Update progress when replicated data(logs or snapshot) matches on follower/learner and is
    /// accepted.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_matching(&mut self, node_id: NID, inflight_id: u64, log_id: Option<LogId<NID>>) {
        tracing::debug!(
            node_id = display(node_id),
            inflight_id = display(inflight_id),
            log_id = display(log_id.summary()),
            "update_progress",
        );
        tracing::debug!(progress = display(&self.leader.progress), "leader progress");

        debug_assert!(log_id.is_some(), "a valid update can never set matching to None");

        // Whether it is a response for the current inflight request.
        let mut is_mine = true;

        // The value granted by a quorum may not yet be a committed.
        // A committed is **granted** and also is in current term.
        let granted = *self
            .leader
            .progress
            .update_with(&node_id, |prog_entry| {
                is_mine = prog_entry.inflight.is_my_id(inflight_id);
                if is_mine {
                    prog_entry.update_matching(log_id);
                }
            })
            .expect("it should always update existing progress");

        if !is_mine {
            return;
        }

        tracing::debug!(granted = display(granted.summary()), "granted after updating progress");

        if node_id != self.config.id {
            // TODO(3): replication metrics should also contains leader's progress
            self.output.push_command(Command::UpdateProgressMetrics {
                target: node_id,
                matching: log_id.unwrap(),
            });
        }

        self.try_commit_granted(granted);
    }

    /// Commit the log id that is granted(accepted) by a quorum of voters.
    ///
    /// In raft a log that is granted and in the leader term is committed.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn try_commit_granted(&mut self, granted: Option<LogId<NID>>) {
        // Only when the log id is proposed by current leader, it is committed.
        if let Some(c) = granted {
            if !self.state.vote_ref().is_same_leader(c.committed_leader_id()) {
                return;
            }
        }

        if let Some(prev_committed) = self.state.update_committed(&granted) {
            self.output.push_command(Command::ReplicateCommitted {
                committed: self.state.committed().copied(),
            });
            self.output.push_command(Command::LeaderCommit {
                already_committed: prev_committed,
                upto: self.state.committed().copied().unwrap(),
            });
        }
    }

    /// Update progress when replicated data(logs or snapshot) does not match follower/learner state
    /// and is rejected.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_conflicting(&mut self, target: NID, inflight_id: u64, conflict: LogId<NID>) {
        // TODO(2): test it?
        tracing::debug!(
            target = display(target),
            inflight_id = display(inflight_id),
            conflict = display(&conflict),
            progress = debug(&self.leader.progress),
            "update_conflicting"
        );

        let prog_entry = self.leader.progress.get_mut(&target).unwrap();

        // Update inflight state only when a matching response is received.
        if !prog_entry.inflight.is_my_id(inflight_id) {
            return;
        }

        prog_entry.update_conflicting(conflict.index);
    }

    /// Update replication progress when a response is received.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_progress(&mut self, target: NID, id: u64, repl_res: Result<ReplicationResult<NID>, String>) {
        // TODO(2): test
        match repl_res {
            Ok(p) => {
                tracing::debug!(id = display(id), result = debug(&p), "update progress");

                match p {
                    ReplicationResult::Matching(matching) => {
                        self.update_matching(target, id, matching);
                    }
                    ReplicationResult::Conflict(conflict) => {
                        self.update_conflicting(target, id, conflict);
                    }
                }
            }
            Err(err_str) => {
                tracing::warn!(id = display(id), result = display(&err_str), "update progress error");

                // Reset inflight state and it will retry.
                let p = self.leader.progress.get_mut(&target).unwrap();

                // Reset inflight state only when a matching response is received.
                if p.inflight.is_my_id(id) {
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
                // TODO:
                tracing::debug!("can not send: TODO");
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

                targets.push((*target, *prog_entry));
            }
        }
        self.output.push_command(Command::RebuildReplicationStreams { targets });
    }

    /// Initiate replication for every target that is not sending data in flight.
    ///
    /// `send_none` specifies whether to force to send a message even when there is no data to send.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn initiate_replication(&mut self, send_none: SendNone) {
        tracing::debug!(progress = debug(&self.leader.progress), "send_to_all");

        for (id, prog_entry) in self.leader.progress.iter_mut() {
            // TODO: update matching should be done here for leader
            //       or updating matching should be queued in commands?
            if id == &self.config.id {
                continue;
            }

            let t = prog_entry.next_send(self.state, self.config.max_payload_entries);

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
    pub(crate) fn send_to_target(output: &mut EngineOutput<NID, N>, target: &NID, inflight: &Inflight<NID>) {
        output.push_command(Command::Replicate {
            target: *target,
            req: *inflight,
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
        let purge_upto = *self.state.purge_upto().unwrap();

        // Check if any replication task is going to use the log that are going to purge.
        let mut in_use = false;
        for (id, prog_entry) in self.leader.progress.iter() {
            if prog_entry.is_inflight(&purge_upto) {
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
    pub(crate) fn update_local_progress(&mut self, upto: Option<LogId<NID>>) {
        if upto.is_none() {
            return;
        }

        let id = self.config.id;

        // The leader may not be in membership anymore
        if let Some(prog_entry) = self.leader.progress.get_mut(&id) {
            if prog_entry.matching >= upto {
                return;
            }
            // TODO: It should be self.state.last_log_id() but None is ok.
            prog_entry.inflight = Inflight::logs(None, upto);

            let inflight_id = prog_entry.inflight.get_id().unwrap();
            self.update_matching(id, inflight_id, upto);
        }
    }

    pub(crate) fn log_handler(&mut self) -> LogHandler<NID, N> {
        LogHandler {
            config: self.config,
            state: self.state,
            output: self.output,
        }
    }
}
