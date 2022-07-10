use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::raft::VoteRequest;
use crate::EffectiveMembership;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MetricsChangeFlags;
use crate::Vote;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn m1() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {1}], None)
}

fn m12() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {1,2}], None)
}

fn eng() -> Engine<u64> {
    Engine::<u64>::default()
}

#[test]
fn test_elect() -> anyhow::Result<()> {
    tracing::info!("--- single node: become leader at once");
    {
        let mut eng = eng();
        eng.id = 1;
        eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(0, 1)), m1()));

        eng.elect();

        assert_eq!(Vote::new_committed(1, 1), eng.state.vote);
        assert_eq!(Some(btreeset! {1},), eng.state.leader.map(|x| x.vote_granted_by));

        assert_eq!(ServerState::Leader, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                leader: false,
                other_metrics: true
            },
            eng.metrics_flags
        );

        assert_eq!(
            vec![
                Command::SaveVote {
                    vote: Vote::new_committed(1, 1)
                },
                Command::UpdateServerState {
                    server_state: ServerState::Leader
                }
            ],
            eng.commands
        );
    }

    tracing::info!("--- single node: electing again will override previous state");
    {
        let mut eng = eng();
        eng.id = 1;
        eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(0, 1)), m1()));

        // Build in-progress election state
        eng.state.vote = Vote::new_committed(1, 2);
        eng.state.new_leader();
        eng.state.leader.as_mut().map(|l| l.vote_granted_by.insert(1));

        eng.elect();

        assert_eq!(Vote::new_committed(2, 1), eng.state.vote);
        assert_eq!(Some(btreeset! {1},), eng.state.leader.map(|x| x.vote_granted_by));

        assert_eq!(ServerState::Leader, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                leader: false,
                other_metrics: true
            },
            eng.metrics_flags
        );

        assert_eq!(
            vec![
                Command::SaveVote {
                    vote: Vote::new_committed(2, 1)
                },
                Command::UpdateServerState {
                    server_state: ServerState::Leader
                }
            ],
            eng.commands
        );
    }

    tracing::info!("--- multi nodes: enter candidate state");
    {
        let mut eng = eng();
        eng.id = 1;
        eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(0, 1)), m12()));
        eng.state.log_ids = LogIdList::new(vec![log_id(1, 1)]);

        eng.elect();

        assert_eq!(Vote::new(1, 1), eng.state.vote);
        assert_eq!(Some(btreeset! {1},), eng.state.leader.map(|x| x.vote_granted_by));

        assert_eq!(ServerState::Candidate, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                leader: false,
                other_metrics: true
            },
            eng.metrics_flags
        );

        assert_eq!(
            vec![
                Command::SaveVote { vote: Vote::new(1, 1) },
                Command::SendVote {
                    vote_req: VoteRequest::new(Vote::new(1, 1), Some(log_id(1, 1)))
                },
                Command::UpdateServerState {
                    server_state: ServerState::Candidate
                }
            ],
            eng.commands
        );
    }
    Ok(())
}
