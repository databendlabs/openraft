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
use crate::raft_state::VoteStateReader;
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
            self.state.get_vote().committed_leader_id().unwrap(),
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
        self.initiate_replication();
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
            old_progress.upgrade_quorum_set(em.membership.to_quorum_set(), &learner_ids, ProgressEntry::empty(end));
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
            if !self.state.get_vote().is_same_leader(c.committed_leader_id()) {
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
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn initiate_replication(&mut self) {
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
                    tracing::debug!("no need to replicate for node-{}: current inflight: {:?}", id, e,);
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_to_target(output: &mut EngineOutput<NID, N>, target: &NID, inflight: &Inflight<NID>) {
        debug_assert!(!inflight.is_none());

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

#[cfg(test)]
mod tests {

    mod update_matching_test {

        use std::sync::Arc;

        use maplit::btreeset;
        use pretty_assertions::assert_eq;

        use crate::engine::Command;
        use crate::engine::Engine;
        use crate::progress::Inflight;
        use crate::progress::Progress;
        use crate::raft_state::LogStateReader;
        use crate::testing::log_id;
        use crate::EffectiveMembership;
        use crate::Membership;
        use crate::MembershipState;
        use crate::Vote;

        fn m01() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
        }

        fn m123() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {1,2,3}], None)
        }

        fn eng() -> Engine<u64, ()> {
            let mut eng = Engine::default();
            eng.state.enable_validate = false; // Disable validation for incomplete state

            eng.config.id = 2;
            eng.state.vote = Vote::new_committed(2, 1);
            eng.state.membership_state = MembershipState::new(
                Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m123())),
            );

            eng
        }

        #[test]
        fn test_update_matching_no_leader() -> anyhow::Result<()> {
            // There is no leader, it should panic.

            let res = std::panic::catch_unwind(move || {
                let mut eng = eng();
                eng.replication_handler().update_matching(3, 0, Some(log_id(1, 2)));
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err());

            Ok(())
        }

        #[test]
        fn test_update_matching() -> anyhow::Result<()> {
            let mut eng = eng();
            eng.vote_handler().become_leading();

            let mut rh = eng.replication_handler();
            let inflight_id_1 = {
                let prog_entry = rh.leader.progress.get_mut(&1).unwrap();
                prog_entry.inflight = Inflight::logs(Some(log_id(2, 3)), Some(log_id(2, 4)));
                prog_entry.inflight.get_id().unwrap()
            };
            let inflight_id_2 = {
                let prog_entry = rh.leader.progress.get_mut(&2).unwrap();
                prog_entry.inflight = Inflight::logs(Some(log_id(1, 0)), Some(log_id(2, 4)));
                prog_entry.inflight.get_id().unwrap()
            };
            let inflight_id_3 = {
                let prog_entry = rh.leader.progress.get_mut(&3).unwrap();
                prog_entry.inflight = Inflight::logs(Some(log_id(1, 1)), Some(log_id(2, 4)));
                prog_entry.inflight.get_id().unwrap()
            };

            // progress: None, None, (1,2)
            {
                rh.update_matching(3, inflight_id_3, Some(log_id(1, 2)));
                assert_eq!(None, rh.state.committed());
                assert_eq!(
                    vec![
                        //
                        Command::UpdateProgressMetrics {
                            target: 3,
                            matching: log_id(1, 2),
                        },
                    ],
                    rh.output.commands
                );
            }

            // progress: None, (2,1), (1,2); quorum-ed: (1,2), not at leader vote, not committed
            {
                rh.output.commands = vec![];
                rh.update_matching(2, inflight_id_2, Some(log_id(2, 1)));
                assert_eq!(None, rh.state.committed());
                assert_eq!(0, rh.output.commands.len());
            }

            // progress: None, (2,1), (2,3); committed: (2,1)
            {
                rh.output.commands = vec![];
                rh.update_matching(3, inflight_id_3, Some(log_id(2, 3)));
                assert_eq!(Some(&log_id(2, 1)), rh.state.committed());
                assert_eq!(
                    vec![
                        Command::UpdateProgressMetrics {
                            target: 3,
                            matching: log_id(2, 3),
                        },
                        Command::ReplicateCommitted {
                            committed: Some(log_id(2, 1))
                        },
                        Command::LeaderCommit {
                            already_committed: None,
                            upto: log_id(2, 1)
                        }
                    ],
                    rh.output.commands
                );
            }

            // progress: (2,4), (2,1), (2,3); committed: (1,3)
            {
                rh.output.commands = vec![];
                rh.update_matching(1, inflight_id_1, Some(log_id(2, 4)));
                assert_eq!(Some(&log_id(2, 3)), rh.state.committed());
                assert_eq!(
                    vec![
                        Command::UpdateProgressMetrics {
                            target: 1,
                            matching: log_id(2, 4),
                        },
                        Command::ReplicateCommitted {
                            committed: Some(log_id(2, 3))
                        },
                        Command::LeaderCommit {
                            already_committed: Some(log_id(2, 1)),
                            upto: log_id(2, 3)
                        }
                    ],
                    rh.output.commands
                );
            }

            Ok(())
        }
    }
    mod append_membership_test {

        use std::sync::Arc;

        use maplit::btreeset;

        use crate::core::ServerState;
        use crate::engine::Command;
        use crate::engine::Engine;
        use crate::engine::LogIdList;
        use crate::progress::entry::ProgressEntry;
        use crate::progress::Inflight;
        use crate::progress::Progress;
        use crate::CommittedLeaderId;
        use crate::EffectiveMembership;
        use crate::LogId;
        use crate::Membership;
        use crate::MembershipState;
        use crate::MetricsChangeFlags;
        use crate::Vote;

        crate::declare_raft_types!(
            pub(crate) Foo: D=(), R=(), NodeId=u64, Node=()
        );

        use crate::testing::log_id;

        fn m01() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
        }

        fn m23() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {2,3}], None)
        }

        fn m23_45() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {2,3}], Some(btreeset! {4,5}))
        }

        fn m34() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {3,4}], None)
        }

        fn m4_356() -> Membership<u64, ()> {
            Membership::<u64, ()>::new(vec![btreeset! {4}], Some(btreeset! {3,5,6}))
        }

        fn eng() -> Engine<u64, ()> {
            let mut eng = Engine::default();
            eng.config.id = 2;
            eng.state.membership_state = MembershipState::new(
                Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
                Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            );
            eng.state.vote = Vote::new_committed(2, 2);
            eng.state.server_state = eng.calc_server_state();
            eng
        }

        #[test]
        fn test_leader_append_membership_for_leader() -> anyhow::Result<()> {
            let mut eng = eng();
            eng.state.server_state = ServerState::Leader;
            // Make it a real leader: voted for itself and vote is committed.
            eng.state.vote = Vote::new_committed(2, 2);
            eng.vote_handler().become_leading();

            eng.replication_handler().append_membership(&log_id(3, 4), &m34());

            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
                    Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m34()))
                ),
                eng.state.membership_state
            );
            assert_eq!(
                ServerState::Leader,
                eng.state.server_state,
                "Leader wont be affected by membership change"
            );

            assert_eq!(
                MetricsChangeFlags {
                    replication: true,
                    local_data: false,
                    cluster: true,
                },
                eng.output.metrics_flags
            );

            assert_eq!(
                vec![
                    //
                    Command::UpdateMembership {
                        membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m34())),
                    },
                    Command::RebuildReplicationStreams {
                        targets: vec![(3, ProgressEntry::empty(0)), (4, ProgressEntry::empty(0))], /* node-2 is leader,
                                                                                                    * won't be removed */
                    }
                ],
                eng.output.commands
            );

            assert!(
                eng.internal_server_state.leading().unwrap().progress.get(&4).matching.is_none(),
                "exists, but it is a None"
            );

            Ok(())
        }

        #[test]
        fn test_leader_append_membership_update_learner_process() -> anyhow::Result<()> {
            // When updating membership, voter progreess should inherit from learner progress, and
            // learner process should inherit from voter process. If voter changes to
            // learner or vice versa.

            let mut eng = eng();
            eng.state.log_ids =
                LogIdList::new([LogId::new(CommittedLeaderId::new(0, 0), 0), log_id(1, 1), log_id(5, 10)]);

            eng.state.server_state = ServerState::Leader;
            // Make it a real leader: voted for itself and vote is committed.
            eng.state.vote = Vote::new_committed(2, 2);
            eng.state
                .membership_state
                .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23_45())));
            eng.vote_handler().become_leading();

            if let Some(l) = &mut eng.internal_server_state.leading_mut() {
                assert_eq!(&ProgressEntry::empty(11), l.progress.get(&4));
                assert_eq!(&ProgressEntry::empty(11), l.progress.get(&5));

                let p = ProgressEntry::new(Some(log_id(1, 4)));
                let _ = l.progress.update(&4, p);
                assert_eq!(&p, l.progress.get(&4));

                let p = ProgressEntry::new(Some(log_id(1, 5)));
                let _ = l.progress.update(&5, p);
                assert_eq!(&p, l.progress.get(&5));

                let p = ProgressEntry::new(Some(log_id(1, 3)));
                let _ = l.progress.update(&3, p);
                assert_eq!(&p, l.progress.get(&3));
            } else {
                unreachable!("leader should not be None");
            }

            eng.replication_handler().append_membership(&log_id(3, 4), &m4_356());

            assert_eq!(
                MembershipState::new(
                    Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23_45())),
                    Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m4_356()))
                ),
                eng.state.membership_state
            );

            if let Some(l) = &mut eng.internal_server_state.leading_mut() {
                assert_eq!(
                    &ProgressEntry::new(Some(log_id(1, 4)))
                        .with_inflight(Inflight::logs(Some(log_id(1, 4)), Some(log_id(5, 10))).with_id(1))
                        .with_curr_inflight_id(1),
                    l.progress.get(&4),
                    "learner-4 progress should be transferred to voter progress"
                );

                assert_eq!(
                    &ProgressEntry::new(Some(log_id(1, 3)))
                        .with_inflight(Inflight::logs(Some(log_id(1, 3)), Some(log_id(5, 10))).with_id(1))
                        .with_curr_inflight_id(1),
                    l.progress.get(&3),
                    "voter-3 progress should be transferred to learner progress"
                );

                assert_eq!(
                    &ProgressEntry::new(Some(log_id(1, 5)))
                        .with_inflight(Inflight::logs(Some(log_id(1, 5)), Some(log_id(5, 10))).with_id(1))
                        .with_curr_inflight_id(1),
                    l.progress.get(&5),
                    "learner-5 has previous value"
                );

                assert_eq!(
                    &ProgressEntry::empty(11)
                        .with_inflight(Inflight::logs(None, Some(log_id(5, 10))).with_id(1))
                        .with_curr_inflight_id(1),
                    l.progress.get(&6)
                );
            } else {
                unreachable!("leader should not be None");
            }

            Ok(())
        }
    }
}
