use std::collections::BTreeSet;
use std::sync::Arc;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::core::ServerState;
use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::entry::RaftEntry;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::raft::VoteResponse;
use crate::raft_state::LogStateReader;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::CommittedLeaderId;
use crate::EffectiveMembership;
use crate::Entry;
use crate::LogId;
use crate::Membership;
use crate::TokioInstant;
use crate::Vote;

fn m12() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2}], None)
}

fn m1234() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2,3,4}], None)
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.state.log_ids = LogIdList::new([LogId::new(CommittedLeaderId::new(0, 0), 0)]);
    eng
}

#[test]
fn test_handle_vote_resp() -> anyhow::Result<()> {
    tracing::info!("--- not in election. just ignore");
    {
        let mut eng = eng();
        eng.state.server_state = ServerState::Follower;
        eng.state.vote = UTime::new(TokioInstant::now(), Vote::new(2, 1));
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m12())));

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 2),
            vote_granted: true,
            last_log_id: Some(log_id(2, 1, 2)),
        });

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert!(eng.internal_server_state.is_following());

        assert_eq!(ServerState::Follower, eng.state.server_state);

        assert_eq!(0, eng.output.take_commands().len());
    }

    tracing::info!("--- recv a smaller vote. vote_granted==false; always keep trying in candidate state");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.vote = UTime::new(TokioInstant::now(), Vote::new(2, 1));
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m12())));
        eng.vote_handler().become_leading();

        let last_log_id = eng.state.last_log_id().copied();

        eng.internal_server_state.leading_mut().map(|l| {
            l.initialize_voting(last_log_id, TokioInstant::now());
            l.voting_mut().unwrap().grant_by(&1)
        });
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(1, 1),
            vote_granted: false,
            last_log_id: Some(log_id(2, 1, 2)),
        });

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(
            Some(btreeset! {1},),
            eng.internal_server_state.leading().map(|x| x.voting().unwrap().granters().collect::<BTreeSet<_>>())
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);

        assert!(eng.output.take_commands().is_empty());
    }

    // TODO: when seeing a higher vote, keep trying until a majority of higher votes are seen.
    tracing::info!("--- seen a higher vote. revert to follower");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.vote = UTime::new(TokioInstant::now(), Vote::new(2, 1));
        eng.state.log_ids = LogIdList::new(vec![log_id(3, 1, 3)]);
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m12())));
        eng.vote_handler().become_leading();

        let last_log_id = eng.state.last_log_id().copied();

        eng.internal_server_state.leading_mut().map(|l| {
            l.initialize_voting(last_log_id, TokioInstant::now());
            l.voting_mut().unwrap().grant_by(&1)
        });
        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(3, 2),
            vote_granted: false,
            last_log_id: Some(log_id(2, 1, 2)),
        });

        assert_eq!(Vote::new(3, 2), *eng.state.vote_ref());
        assert!(eng.internal_server_state.is_following());

        assert_eq!(ServerState::Follower, eng.state.server_state);

        assert_eq!(
            vec![Command::SaveVote { vote: Vote::new(3, 2) },],
            eng.output.take_commands()
        );
    }

    tracing::info!("--- equal vote, rejected by higher last_log_id. keep trying in candidate state");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.vote = UTime::new(TokioInstant::now(), Vote::new(2, 1));
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m12())));
        eng.vote_handler().become_leading();

        let last_log_id = eng.state.last_log_id().copied();

        eng.internal_server_state.leading_mut().map(|l| {
            l.initialize_voting(last_log_id, TokioInstant::now());
            l.voting_mut().unwrap().grant_by(&1)
        });

        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: false,
            last_log_id: Some(log_id(2, 1, 2)),
        });

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(
            Some(btreeset! {1},),
            eng.internal_server_state.leading().map(|x| x.voting().unwrap().granters().collect::<BTreeSet<_>>())
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);

        assert!(eng.output.take_commands().is_empty());
    }

    tracing::info!("--- equal vote, granted, but not constitute a quorum. nothing to do");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.vote = UTime::new(TokioInstant::now(), Vote::new(2, 1));
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m1234())));
        eng.vote_handler().become_leading();

        let last_log_id = eng.state.last_log_id().copied();

        eng.internal_server_state.leading_mut().map(|l| {
            l.initialize_voting(last_log_id, TokioInstant::now());
            l.voting_mut().unwrap().grant_by(&1)
        });

        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: true,
            last_log_id: Some(log_id(2, 1, 2)),
        });

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(
            Some(btreeset! {1,2},),
            eng.internal_server_state.leading().map(|x| x.voting().unwrap().granters().collect::<BTreeSet<_>>())
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);

        assert_eq!(0, eng.output.take_commands().len());
    }

    tracing::info!("--- equal vote, granted, constitute a quorum. become leader");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.vote = UTime::new(TokioInstant::now(), Vote::new(2, 1));
        eng.state
            .membership_state
            .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m12())));
        eng.vote_handler().become_leading();

        let last_log_id = eng.state.last_log_id().copied();

        eng.internal_server_state.leading_mut().map(|l| {
            l.initialize_voting(last_log_id, TokioInstant::now());
            l.voting_mut().unwrap().grant_by(&1)
        });

        eng.state.server_state = ServerState::Candidate;

        eng.handle_vote_resp(2, VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: true,
            last_log_id: Some(log_id(2, 1, 2)),
        });

        assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
        assert!(
            eng.internal_server_state.voting_mut().is_none(),
            "voting state is removed when becoming leader"
        );

        assert_eq!(ServerState::Leader, eng.state.server_state);

        assert_eq!(
            vec![
                Command::SaveVote {
                    vote: Vote::new_committed(2, 1)
                },
                Command::BecomeLeader,
                Command::RebuildReplicationStreams {
                    targets: vec![(2, ProgressEntry::empty(1))]
                },
                Command::AppendEntry {
                    entry: Entry::<UTConfig>::new_blank(log_id(2, 1, 1)),
                },
                Command::Replicate {
                    target: 2,
                    req: Inflight::logs(None, Some(log_id(2, 1, 1))).with_id(1),
                },
            ],
            eng.output.take_commands()
        );
    }

    Ok(())
}
