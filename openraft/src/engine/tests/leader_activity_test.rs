use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use crate::Membership;
use crate::MembershipState;
use crate::ServerState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::proposer::leader_activity::LeaderActivity;
use crate::raft::VoteResponse;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

const PROBE_INTERVAL: Duration = Duration::from_millis(700);

pub(super) fn engine(enabled: bool) -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false);
    let membership = Membership::new_with_defaults(vec![btreeset! {0, 1, 2, 3, 4}], [5]);
    let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership));
    eng.state.membership_state = MembershipState::new(stored.clone(), stored);
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::ZERO, Vote::new_committed(1, 0));
    eng.state.log_ids.append(log_id(1, 0, 1));
    eng.state.server_state = ServerState::Leader;
    eng.config.quorum_loss_probe_interval = enabled.then_some(PROBE_INTERVAL);
    eng.testing_new_leader();
    eng
}

#[test]
fn activity_is_initialized_only_when_enabled() {
    let mut disabled = engine(false);
    disabled.handle_leader_activity(UTConfig::<()>::now() + Duration::from_secs(1));
    assert_eq!(disabled.leader.as_ref().unwrap().activity, None);
    assert!(disabled.output.take_commands().is_empty());

    let before = UTConfig::<()>::now();
    let enabled = engine(true);
    let after = UTConfig::<()>::now();
    let lease = enabled.config.timer_config.leader_lease;
    let Some(LeaderActivity::Active { next_check_at }) = enabled.leader.as_ref().unwrap().activity else {
        panic!("enabled policy must initialize active Leader state");
    };
    assert!(before + lease <= next_check_at && next_check_at <= after + lease);
}

#[test]
fn election_initializes_activity_when_enabled() {
    let mut eng: Engine<UTConfig> = Engine::testing_default(0);
    eng.state.enable_validation(false);
    let membership = Membership::new_with_defaults(vec![btreeset! {0, 1, 2}], []);
    let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership));
    eng.state.membership_state = MembershipState::new(stored.clone(), stored);
    eng.config.quorum_loss_probe_interval = Some(PROBE_INTERVAL);

    tracing::info!("Elect a Leader through the normal candidate path");
    {
        eng.elect();
        let vote = *eng.state.vote_ref();
        eng.handle_vote_resp(0, VoteResponse::new(vote, None, true));

        let before = UTConfig::<()>::now();
        eng.handle_vote_resp(1, VoteResponse::new(vote, None, true));
        let after = UTConfig::<()>::now();

        let lease = eng.config.timer_config.leader_lease;
        let Some(LeaderActivity::Active { next_check_at }) = eng.leader.as_ref().unwrap().activity else {
            panic!("an elected Leader must initialize activity tracking");
        };
        assert!(before + lease <= next_check_at && next_check_at <= after + lease);
    }
}

#[test]
fn due_activity_check_closes_admission_and_schedules_probes() {
    let mut eng = engine(true);
    let Some(LeaderActivity::Active { next_check_at }) = eng.leader.as_ref().unwrap().activity else {
        unreachable!();
    };

    tracing::info!("Keep admission open until the activity deadline");
    {
        eng.handle_leader_activity(next_check_at - Duration::from_millis(1));
        assert!(eng.leader.as_ref().unwrap().is_active());
        assert!(eng.output.take_commands().is_empty());
    }

    tracing::info!("Close admission when the deadline has no fresh quorum evidence");
    let next_probe_at = {
        eng.handle_leader_activity(next_check_at);
        assert!(matches!(eng.output.take_commands().as_slice(), [
            Command::SetLeaderActivity { active: false, .. }
        ]));
        let Some(LeaderActivity::Inactive { next_probe_at }) = eng.leader.as_ref().unwrap().activity else {
            panic!("failed activity check must make the Leader inactive");
        };
        assert_eq!(next_check_at + PROBE_INTERVAL, next_probe_at);
        next_probe_at
    };

    tracing::info!("Request a quorum probe only when its deadline is due");
    {
        eng.handle_leader_activity(next_probe_at - Duration::from_millis(1));
        assert!(eng.output.take_commands().is_empty());

        eng.handle_leader_activity(next_probe_at);
        assert!(matches!(eng.output.take_commands().as_slice(), [
            Command::QuorumProbe { .. }
        ]));

        eng.schedule_next_quorum_probe(next_probe_at);
        assert!(!eng.leader.as_ref().unwrap().is_quorum_probe_due(next_probe_at));
    }
}

#[test]
fn responses_recover_inactive_admission_and_refresh_active_deadline() {
    let mut eng = engine(true);
    let now = UTConfig::<()>::now();
    eng.leader.as_mut().unwrap().activity = Some(LeaderActivity::Inactive {
        next_probe_at: now + PROBE_INTERVAL,
    });

    tracing::info!("Keep admission closed until responses form a fresh voter quorum");
    {
        let stream = eng.leader.as_ref().unwrap().progress.try_get(&1).unwrap().data.stream_id;
        eng.replication_handler().try_update_leader_clock(stream, 1, now);
        assert!(eng.leader.as_ref().unwrap().is_inactive());
        assert!(eng.output.take_commands().is_empty());

        let stream = eng.leader.as_ref().unwrap().progress.try_get(&2).unwrap().data.stream_id;
        eng.replication_handler().try_update_leader_clock(stream, 2, now);
        assert_eq!(
            Some(LeaderActivity::Active {
                next_check_at: now + eng.config.timer_config.leader_lease,
            }),
            eng.leader.as_ref().unwrap().activity
        );
        assert!(matches!(eng.output.take_commands().as_slice(), [
            Command::SetLeaderActivity { active: true, .. }
        ]));
    }

    tracing::info!("Fresh responses extend an active deadline without another activity command");
    {
        eng.leader.as_mut().unwrap().activity = Some(LeaderActivity::Active { next_check_at: now });
        let stream = eng.leader.as_ref().unwrap().progress.try_get(&1).unwrap().data.stream_id;
        eng.replication_handler().try_update_leader_clock(stream, 1, now);
        assert_eq!(
            Some(LeaderActivity::Active {
                next_check_at: now + eng.config.timer_config.leader_lease,
            }),
            eng.leader.as_ref().unwrap().activity
        );
        assert!(eng.output.take_commands().is_empty());
    }
}
