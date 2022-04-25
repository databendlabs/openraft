use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::leader::Leader;
use crate::raft::VoteResponse;
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

fn m12() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {1,2}], None)
}

fn m1234() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {1,2,3,4}], None)
}

fn eng() -> Engine<u64> {
    Engine::<u64>::default()
}

#[test]
fn test_handle_vote_resp() -> anyhow::Result<()> {
    tracing::info!("--- not in election. just ignore");
    {
        let mut eng = eng();
        eng.state.vote = Vote::new(2, 1);
        eng.state.effective_membership = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12()));

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 2),
            vote_granted: true,
            last_log_id: Some(log_id(2, 2)),
        });
        eng.update_metrics_flags();

        assert_eq!(Vote::new(2, 1), eng.state.vote);
        assert_eq!(None, eng.state.leader);

        assert_eq!(ServerState::Follower, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                leader: false,
                other_metrics: false
            },
            eng.metrics_flags
        );

        assert_eq!(0, eng.commands.len());
    }

    tracing::info!("--- recv an old resp. just ignore");
    {
        let mut eng = eng();
        eng.id = 1;
        eng.state.vote = Vote::new(2, 1);
        eng.state.leader = Some(Leader {
            vote_granted_by: btreeset! {1},
        });
        eng.state.effective_membership = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12()));
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(1, 1),
            vote_granted: true,
            last_log_id: Some(log_id(2, 2)),
        });
        eng.update_metrics_flags();

        assert_eq!(Vote::new(2, 1), eng.state.vote);
        assert_eq!(
            Some(Leader {
                vote_granted_by: btreeset! {1}
            }),
            eng.state.leader
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                leader: false,
                other_metrics: false
            },
            eng.metrics_flags
        );

        assert_eq!(0, eng.commands.len());
    }

    tracing::info!("--- seen a higher vote. revert to follower");
    {
        let mut eng = eng();
        eng.id = 1;
        eng.state.vote = Vote::new(2, 1);
        eng.state.leader = Some(Leader {
            vote_granted_by: btreeset! {1},
        });
        eng.state.effective_membership = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12()));
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 2),
            vote_granted: true,
            last_log_id: Some(log_id(2, 2)),
        });
        eng.update_metrics_flags();

        assert_eq!(Vote::new(2, 2), eng.state.vote);
        assert_eq!(None, eng.state.leader);

        assert_eq!(ServerState::Follower, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                leader: false,
                other_metrics: true
            },
            eng.metrics_flags
        );

        assert_eq!(
            vec![
                Command::UpdateServerState {
                    server_state: ServerState::Follower
                },
                Command::SaveVote { vote: Vote::new(2, 2) }
            ],
            eng.commands
        );
    }

    tracing::info!("--- equal vote, rejected by higher last_log_id. nothing to do");
    {
        let mut eng = eng();
        eng.id = 1;
        eng.state.vote = Vote::new(2, 1);
        eng.state.leader = Some(Leader {
            vote_granted_by: btreeset! {1},
        });
        eng.state.effective_membership = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12()));
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: false,
            last_log_id: Some(log_id(2, 2)),
        });
        eng.update_metrics_flags();

        assert_eq!(Vote::new(2, 1), eng.state.vote);
        assert_eq!(
            Some(Leader {
                vote_granted_by: btreeset! {1}
            }),
            eng.state.leader
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                leader: false,
                other_metrics: false
            },
            eng.metrics_flags
        );

        assert_eq!(0, eng.commands.len());
    }

    tracing::info!("--- equal vote, granted, but not constitute a quorum. nothing to do");
    {
        let mut eng = eng();
        eng.id = 1;
        eng.state.vote = Vote::new(2, 1);
        eng.state.leader = Some(Leader {
            vote_granted_by: btreeset! {1},
        });
        eng.state.effective_membership = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234()));
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: true,
            last_log_id: Some(log_id(2, 2)),
        });
        eng.update_metrics_flags();

        assert_eq!(Vote::new(2, 1), eng.state.vote);
        assert_eq!(
            Some(Leader {
                vote_granted_by: btreeset! {1,2}
            }),
            eng.state.leader
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                leader: false,
                other_metrics: false
            },
            eng.metrics_flags
        );

        assert_eq!(0, eng.commands.len());
    }

    tracing::info!("--- equal vote, granted, constitute a quorum. become leader");
    {
        let mut eng = eng();
        eng.id = 1;
        eng.state.vote = Vote::new(2, 1);
        eng.state.leader = Some(Leader {
            vote_granted_by: btreeset! {1},
        });
        eng.state.effective_membership = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12()));
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: true,
            last_log_id: Some(log_id(2, 2)),
        });
        eng.update_metrics_flags();

        assert_eq!(Vote::new_committed(2, 1), eng.state.vote);
        assert_eq!(
            Some(Leader {
                vote_granted_by: btreeset! {1,2}
            }),
            eng.state.leader
        );

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

    Ok(())
}
