use std::sync::Arc;

use maplit::btreeset;
use pretty_assertions::assert_eq;
use tokio::time::Instant;

use crate::core::ServerState;
use crate::engine::testing::UTCfg;
use crate::engine::CEngine;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::raft::VoteResponse;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::CommittedLeaderId;
use crate::EffectiveMembership;
use crate::LogId;
use crate::Membership;
use crate::MetricsChangeFlags;
use crate::Vote;

fn m12() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2}], None)
}

fn m1234() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2,3,4}], None)
}

fn eng() -> CEngine<UTCfg> {
    let mut eng = Engine::default();
    eng.state.enable_validate = false; // Disable validation for incomplete state

    eng.state.log_ids = LogIdList::new([LogId::new(CommittedLeaderId::new(0, 0), 0)]);
    eng
}

#[test]
fn test_handle_vote_resp() -> anyhow::Result<()> {
    tracing::info!("--- not in election. just ignore");
    {
        let mut eng = eng();
        eng.state.server_state = ServerState::Follower;
        eng.state.vote = UTime::new(Instant::now(), Vote::new(2, 1));
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12())));

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 2),
            vote_granted: true,
            last_log_id: Some(log_id(2, 2)),
        });

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert!(eng.internal_server_state.is_following());

        assert_eq!(ServerState::Follower, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                replication: false,
                local_data: false,
                cluster: false,
            },
            eng.output.metrics_flags
        );

        assert_eq!(0, eng.output.take_commands().len());
    }

    tracing::info!("--- recv a smaller vote. vote_granted==false always; keep trying in candidate state");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.vote = UTime::new(Instant::now(), Vote::new(2, 1));
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12())));
        eng.vote_handler().become_leading();
        eng.internal_server_state.leading_mut().map(|l| l.vote_granted_by.insert(1));
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(1, 1),
            vote_granted: false,
            last_log_id: Some(log_id(2, 2)),
        });

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(
            Some(btreeset! {1},),
            eng.internal_server_state.leading().map(|x| x.vote_granted_by.clone())
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                replication: false,
                local_data: false,
                cluster: false,
            },
            eng.output.metrics_flags
        );

        assert!(eng.output.take_commands().is_empty());
    }

    // TODO: when seeing a higher vote, keep trying until a majority of higher votes are seen.
    tracing::info!("--- seen a higher vote. revert to follower");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.vote = UTime::new(Instant::now(), Vote::new(2, 1));
        eng.state.log_ids = LogIdList::new(vec![log_id(3, 3)]);
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12())));
        eng.vote_handler().become_leading();
        eng.internal_server_state.leading_mut().map(|l| l.vote_granted_by.insert(1));
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(3, 2),
            vote_granted: false,
            last_log_id: Some(log_id(2, 2)),
        });

        assert_eq!(Vote::new(3, 2), *eng.state.vote_ref());
        assert!(eng.internal_server_state.is_following());

        assert_eq!(ServerState::Follower, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                replication: false,
                local_data: true,
                cluster: false,
            },
            eng.output.metrics_flags
        );

        assert_eq!(
            vec![Command::SaveVote { vote: Vote::new(3, 2) },],
            eng.output.take_commands()
        );
    }

    tracing::info!("--- equal vote, rejected by higher last_log_id. keep trying in candidate state");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.vote = UTime::new(Instant::now(), Vote::new(2, 1));
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12())));
        eng.vote_handler().become_leading();
        eng.internal_server_state.leading_mut().map(|l| l.vote_granted_by.insert(1));
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: false,
            last_log_id: Some(log_id(2, 2)),
        });

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(
            Some(btreeset! {1},),
            eng.internal_server_state.leading().map(|x| x.vote_granted_by.clone())
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                replication: false,
                local_data: false,
                cluster: false,
            },
            eng.output.metrics_flags
        );

        assert!(eng.output.take_commands().is_empty());
    }

    tracing::info!("--- equal vote, granted, but not constitute a quorum. nothing to do");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.vote = UTime::new(Instant::now(), Vote::new(2, 1));
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())));
        eng.vote_handler().become_leading();
        eng.internal_server_state.leading_mut().map(|l| l.vote_granted_by.insert(1));
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: true,
            last_log_id: Some(log_id(2, 2)),
        });

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(
            Some(btreeset! {1,2},),
            eng.internal_server_state.leading().map(|x| x.vote_granted_by.clone())
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                replication: false,
                local_data: false,
                cluster: false,
            },
            eng.output.metrics_flags
        );

        assert_eq!(0, eng.output.take_commands().len());
    }

    tracing::info!("--- equal vote, granted, constitute a quorum. become leader");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.vote = UTime::new(Instant::now(), Vote::new(2, 1));
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m12())));
        eng.vote_handler().become_leading();
        eng.internal_server_state.leading_mut().map(|l| l.vote_granted_by.insert(1));
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: true,
            last_log_id: Some(log_id(2, 2)),
        });

        assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
        assert_eq!(
            Some(btreeset! {1,2},),
            eng.internal_server_state.leading().map(|x| x.vote_granted_by.clone())
        );

        assert_eq!(ServerState::Leader, eng.state.server_state);
        assert_eq!(
            MetricsChangeFlags {
                replication: true,
                local_data: true,
                cluster: true,
            },
            eng.output.metrics_flags
        );

        assert_eq!(
            vec![
                Command::SaveVote {
                    vote: Vote::new_committed(2, 1)
                },
                Command::BecomeLeader,
                Command::RebuildReplicationStreams {
                    targets: vec![(2, ProgressEntry::empty(1))]
                },
                Command::AppendBlankLog {
                    log_id: LogId {
                        leader_id: CommittedLeaderId::new(2, 1),
                        index: 1,
                    },
                },
                Command::Replicate {
                    target: 2,
                    req: Inflight::logs(None, Some(log_id(2, 1))).with_id(1),
                },
            ],
            eng.output.take_commands()
        );
    }

    Ok(())
}
