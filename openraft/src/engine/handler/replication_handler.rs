use std::ops::Deref;

use crate::engine::engine_impl::EngineOutput;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::internal_server_state::LeaderQuorumSet;
use crate::leader::Leader;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::replication::ReplicationResult;
use crate::LogId;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RaftState;

/// Handle raft vote related operations
pub(crate) struct ReplicationHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) config: &'x EngineConfig<NID>,
    pub(crate) leader: &'x mut Leader<NID, LeaderQuorumSet<NID>>,
    pub(crate) state: &'x mut RaftState<NID, N>,
    pub(crate) output: &'x mut EngineOutput<NID, N>,
}

impl<'x, NID, N> ReplicationHandler<'x, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Update progress when replicated data(logs or snapshot) matches on follower/learner and is accepted.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_matching(&mut self, node_id: NID, inflight_id: u64, log_id: Option<LogId<NID>>) {
        tracing::debug!(
            node_id = display(node_id),
            inflight_id = display(inflight_id),
            log_id = display(log_id.summary()),
            "update_progress",
        );
        tracing::debug!(progress = debug(&self.leader.progress), "leader progress");

        let mut prog_entry = *self.leader.progress.try_get(&node_id).expect("should be a progress");

        // Update inflight state only when a matching response is received.
        if !prog_entry.inflight.is_my_id(inflight_id) {
            return;
        }

        prog_entry.update_matching(log_id);

        let committed = *self
            .leader
            .progress
            .update(&node_id, prog_entry)
            .expect("it should always update existing progress");

        tracing::debug!(committed = debug(&committed), "committed after updating progress");

        debug_assert!(log_id.is_some(), "a valid update can never set matching to None");

        if node_id != self.config.id {
            self.output.push_command(Command::UpdateReplicationMetrics {
                target: node_id,
                matching: log_id.unwrap(),
            });
        }

        // Only when the log id is proposed by current leader, it is committed.
        if let Some(c) = committed {
            if c.leader_id.term != self.state.vote.term || c.leader_id.node_id != self.state.vote.node_id {
                return;
            }
        }

        if let Some(prev_committed) = self.state.update_committed(&committed) {
            self.output.push_command(Command::ReplicateCommitted {
                committed: self.state.committed,
            });
            self.output.push_command(Command::LeaderCommit {
                already_committed: prev_committed,
                upto: self.state.committed.unwrap(),
            });
        }
    }

    /// Update progress when replicated data(logs or snapshot) does not match follower/learner state and is rejected.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_conflicting(&mut self, target: NID, inflight_id: u64, conflict: LogId<NID>) {
        // TODO(1): test it?
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
        // TODO(1): test
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
                Self::send_to(self.output, &target, inflight);
            } else {
                // TODO:
                tracing::debug!("can not send: TODO");
            }
        }
    }

    /// Update replication streams to reflect replication progress change.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_replications(&mut self) {
        let mut targets = vec![];

        // TODO: maybe it's better to update leader's matching when update_repliation() is called.
        for (target, prog_entry) in self.leader.progress.iter_mut() {
            if target != &self.config.id {
                // Reset and resend(by self.send_to_all()) replication requests.
                prog_entry.inflight = Inflight::None;

                targets.push((*target, *prog_entry));
            }
        }
        self.output.push_command(Command::UpdateReplicationStreams { targets });
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_to_all(&mut self) {
        tracing::debug!(progress = debug(&self.leader.progress), "send_to_all");
        // TODO(1):

        for (id, prog_entry) in self.leader.progress.iter_mut() {
            // TODO: update matching should be done here for leader
            //       or updating matching should be queued in commands?
            if id == &self.config.id {
                continue;
            }

            let t = prog_entry.next_send(self.state, self.config.max_payload_entries);

            match t {
                Ok(inflight) => {
                    Self::send_to(self.output, id, inflight);
                }
                Err(e) => {
                    tracing::debug!("no need to replicate for node-{}: current inflight: {:?}", id, e,);
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_to(output: &mut EngineOutput<NID, N>, node_id: &NID, inflight: &Inflight<NID>) {
        // TODO(1): test

        debug_assert!(!inflight.is_none());

        output.push_command(Command::ReplicateEnt {
            target: *node_id,
            req: *inflight,
        });
    }

    /// Try to run a pending purge job, if no tasks are using the logs to be purged.
    ///
    /// Purging logs involves concurrent log accesses by replication tasks and purging task.
    /// Therefor it is a method of ReplicationHandler.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn try_purge_log(&mut self) {
        // TODO: test
        let purge_upto = if let Some(x) = self.state.want_to_purge.take() {
            tracing::debug!("pending purge: {}", x.summary());
            x
        } else {
            tracing::debug!("no pending purge");
            return;
        };

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
            // TODO: when update replications, inflight is cleaned, thus new replication must be closed completed
            //       before purging.
            // TODO: next_send should never try to send log that are scheduled to purge.
            debug_assert!(self.state.want_to_purge <= Some(purge_upto));
            tracing::debug!("can not purge, postpone purge: {}", purge_upto);
            self.state.want_to_purge = Some(purge_upto);
        } else {
            tracing::debug!("do purge: {}", purge_upto.summary());
            self.log_handler().purge_log(purge_upto);
        }
    }

    pub(crate) fn log_handler(&mut self) -> LogHandler<NID, N> {
        LogHandler {
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
        use crate::EffectiveMembership;
        use crate::LeaderId;
        use crate::LogId;
        use crate::Membership;
        use crate::Vote;

        fn log_id(term: u64, index: u64) -> LogId<u64> {
            LogId::<u64> {
                leader_id: LeaderId { term, node_id: 1 },
                index,
            }
        }

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
            eng.state.membership_state.committed = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
            eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m123()));
            eng
        }

        #[test]
        fn test_update_matching_no_leader() -> anyhow::Result<()> {
            let mut eng = eng();

            // There is no leader, it should panic.

            let res = std::panic::catch_unwind(move || {
                eng.replication_handler().update_matching(3, 0, Some(log_id(1, 2)));
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err());

            Ok(())
        }

        #[test]
        fn test_update_matching_update_leader_progress() -> anyhow::Result<()> {
            let mut eng = eng();
            eng.new_leader();

            // TODO(1): If inflight becomes None, it should output another replication action.
            //          Otherwise, it outputs nothing.
            let rh = eng.replication_handler();
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
                eng.replication_handler().update_matching(3, inflight_id_3, Some(log_id(1, 2)));
                assert_eq!(None, eng.state.committed);
                assert_eq!(
                    vec![
                        //
                        Command::UpdateReplicationMetrics {
                            target: 3,
                            matching: log_id(1, 2),
                        },
                    ],
                    eng.output.commands
                );
            }

            // progress: None, (2,1), (1,2); quorum-ed: (1,2), not at leader vote, not committed
            {
                eng.output.commands = vec![];
                eng.replication_handler().update_matching(2, inflight_id_2, Some(log_id(2, 1)));
                assert_eq!(None, eng.state.committed);
                assert_eq!(0, eng.output.commands.len());
            }

            // progress: None, (2,1), (2,3); committed: (2,1)
            {
                eng.output.commands = vec![];
                eng.replication_handler().update_matching(3, inflight_id_3, Some(log_id(2, 3)));
                assert_eq!(Some(log_id(2, 1)), eng.state.committed);
                assert_eq!(
                    vec![
                        Command::UpdateReplicationMetrics {
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
                    eng.output.commands
                );
            }

            // progress: (2,4), (2,1), (2,3); committed: (1,3)
            {
                eng.output.commands = vec![];
                eng.replication_handler().update_matching(1, inflight_id_1, Some(log_id(2, 4)));
                assert_eq!(Some(log_id(2, 3)), eng.state.committed);
                assert_eq!(
                    vec![
                        Command::UpdateReplicationMetrics {
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
                    eng.output.commands
                );
            }

            Ok(())
        }
    }
}
