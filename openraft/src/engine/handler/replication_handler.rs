use crate::engine::engine_impl::EngineOutput;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::internal_server_state::LeaderQuorumSet;
use crate::leader::Leader;
use crate::progress::Progress;
use crate::LogId;
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
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn update_progress(&mut self, node_id: NID, log_id: Option<LogId<NID>>) {
        tracing::debug!("update_progress: node_id:{} log_id:{:?}", node_id, log_id);
        tracing::debug!(progress = debug(&self.leader.progress), "leader progress");

        let v = self.leader.progress.try_get(&node_id);
        let mut updated = match v {
            None => {
                // TODO no such node, it should be a bug.
                return;
            }
            Some(x) => *x,
        };

        updated.update_matching(log_id);

        let res = self.leader.progress.update(&node_id, updated);
        let committed = match res {
            Ok(c) => *c,
            Err(_) => {
                // TODO: leader should not append log if it is no longer in the membership.
                //       There is a chance this will happen:
                //       If leader is `1`, when a the membership changes from [1,2,3] to [2,3],
                //       The leader will still try to append log to its local store.
                //       This is still correct but unnecessary.
                //       To make thing clear, a leader should stop appending log at once if it is no longer in the
                //       membership.
                //       The replication task should be generalized to write log for
                //       both leader and follower.

                // unreachable!("updating nonexistent id: {}, progress: {:?}", node_id, leader.progress);

                return;
            }
        };

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
}

#[cfg(test)]
mod tests {

    mod update_progress_test {

        use std::sync::Arc;

        use maplit::btreeset;
        use pretty_assertions::assert_eq;

        use crate::engine::Command;
        use crate::engine::Engine;
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
        fn test_update_progress_no_leader() -> anyhow::Result<()> {
            let mut eng = eng();

            // There is no leader, it should panic.

            let res = std::panic::catch_unwind(move || {
                eng.replication_handler().update_progress(3, Some(log_id(1, 2)));
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err());

            Ok(())
        }

        #[test]
        fn test_update_progress_update_leader_progress() -> anyhow::Result<()> {
            let mut eng = eng();
            eng.new_leader();

            // progress: None, None, (1,2)
            eng.replication_handler().update_progress(3, Some(log_id(1, 2)));
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

            // progress: None, (2,1), (1,2); quorum-ed: (1,2), not at leader vote, not committed
            eng.output.commands = vec![];
            eng.replication_handler().update_progress(2, Some(log_id(2, 1)));
            assert_eq!(None, eng.state.committed);
            assert_eq!(0, eng.output.commands.len());

            // progress: None, (2,1), (2,3); committed: (2,1)
            eng.output.commands = vec![];
            eng.replication_handler().update_progress(3, Some(log_id(2, 3)));
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

            eng.output.commands = vec![];
            // progress: (2,4), (2,1), (2,3); committed: (1,3)
            eng.replication_handler().update_progress(1, Some(log_id(2, 4)));
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

            Ok(())
        }
    }
}
