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
use crate::progress::Inflight;
use crate::progress::stream_id::StreamId;
use crate::proposer::leader_activity::LeaderActivity;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

pub(super) fn engine(enabled: bool) -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false);
    let membership = Membership::new_with_defaults(vec![btreeset! {0, 1, 2, 3, 4}], [5]);
    let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership));
    eng.state.membership_state = MembershipState::new(stored.clone(), stored);
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::ZERO, Vote::new_committed(1, 0));
    eng.state.log_ids.append(log_id(1, 0, 1));
    eng.state.server_state = ServerState::Leader;
    if enabled {
        eng.config.quorum_loss_grace = Some(Duration::from_millis(20));
        eng.config.quorum_loss_probe_interval = Some(Duration::from_millis(700));
    }
    eng.testing_new_leader();
    eng
}

#[test]
fn inactive_commands_precede_waiting_io_without_reserving_replication() {
    let mut eng = engine(true);
    let now = UTConfig::<()>::now();
    let interval = eng.config.quorum_loss_probe_interval.unwrap();
    eng.leader.as_mut().unwrap().activity = Some(LeaderActivity::AwaitingQuorum { inactive_at: now });
    eng.output.push_command(Command::SaveVote { vote: Vote::new(2, 0) });

    tracing::info!("Place admission closure before earlier queued work without changing Leader authority");
    {
        eng.handle_leader_activity(now);
        let commands = eng.output.take_commands();
        assert!(
            matches!(&commands[0], Command::SetLeaderActivity { leader_vote, active: false }
            if leader_vote == &Vote::new(1, 0).to_committed())
        );
        assert!(matches!(&commands[1], Command::SaveVote { .. }));
        assert_eq!(eng.state.server_state, ServerState::Leader);
        assert_eq!(eng.state.vote_ref(), &Vote::new_committed(1, 0));
        eng.replication_handler().initiate_replication();
        eng.try_leader_handler().unwrap().send_heartbeat(true);
        assert!(eng.output.take_commands().is_empty());
        assert!(
            eng.leader
                .as_ref()
                .unwrap()
                .progress
                .iter()
                .all(|entry| matches!(entry.data.inflight, Inflight::None))
        );
    }

    tracing::info!("Creating a due probe does not advance its deadline; actual submission does");
    {
        let closed_at = now + Duration::from_millis(50);
        eng.arm_quorum_probe(closed_at);
        eng.handle_leader_activity(closed_at + interval - Duration::from_millis(1));
        assert!(eng.output.take_commands().is_empty());
        let due = closed_at + interval;
        eng.handle_leader_activity(due);
        assert!(matches!(eng.output.take_commands().as_slice(), [
            Command::QuorumProbe { .. }
        ]));
        assert!(eng.leader.as_ref().unwrap().is_quorum_probe_due(due));
        let submitted_at = due + Duration::from_millis(50);
        eng.arm_quorum_probe(submitted_at);
        assert!(!eng.leader.as_ref().unwrap().is_quorum_probe_due(submitted_at + interval - Duration::from_millis(1)));
        eng.handle_leader_activity(submitted_at + interval);
        assert!(matches!(eng.output.take_commands().as_slice(), [
            Command::QuorumProbe { .. }
        ]));
    }
}

#[test]
fn validated_fresh_responses_recover_without_tick() {
    let mut eng = engine(true);
    let now = UTConfig::<()>::now();
    let deadline = now + Duration::from_millis(700);
    eng.leader.as_mut().unwrap().activity = Some(LeaderActivity::Inactive {
        next_probe_at: deadline,
    });

    tracing::info!("Ignore stale streams, learners, and an incomplete voter quorum");
    {
        let stream = eng.leader.as_ref().unwrap().progress.try_get(&1).unwrap().data.stream_id;
        eng.replication_handler().try_update_leader_clock(StreamId::new(*stream + 100), 1, now);
        assert_eq!(
            eng.leader.as_ref().unwrap().clock_progress.try_get(&1).unwrap().val,
            None
        );
        eng.replication_handler().try_update_leader_clock(stream, 1, now);
        let learner_stream = eng.leader.as_ref().unwrap().progress.try_get(&5).unwrap().data.stream_id;
        eng.replication_handler().try_update_leader_clock(learner_stream, 5, now);
        assert!(eng.leader.as_ref().unwrap().is_inactive());
        assert!(eng.output.take_commands().is_empty());
    }

    tracing::info!("A complete fresh quorum immediately opens admission without a new observation window");
    {
        let stream = eng.leader.as_ref().unwrap().progress.try_get(&2).unwrap().data.stream_id;
        eng.replication_handler().try_update_leader_clock(stream, 2, now);
        assert_eq!(
            eng.leader.as_ref().unwrap().activity,
            Some(LeaderActivity::Active { observe_until: None })
        );
        assert!(matches!(eng.output.take_commands().as_slice(), [
            Command::SetLeaderActivity { active: true, .. }
        ]));
        eng.replication_handler().initiate_replication();
        assert!(
            !eng.output.take_commands().is_empty(),
            "Idle targets resume through normal replication"
        );
        eng.replication_handler().initiate_replication();
        assert!(
            eng.output.take_commands().is_empty(),
            "Existing inflight work must not be duplicated"
        );
    }
}

#[test]
fn disabled_policy_retains_normal_commands_and_has_no_deadlines() {
    let mut eng = engine(false);
    let now = UTConfig::<()>::now();

    tracing::info!("Disabled policy leaves the controller absent and ordinary traffic available");
    {
        eng.config.quorum_loss_probe_interval = Some(Duration::ZERO);
        eng.handle_leader_activity(now + Duration::from_secs(100));
        eng.arm_quorum_probe(now);
        assert_eq!(eng.leader.as_ref().unwrap().activity, None);
        assert!(eng.output.take_commands().is_empty());
        eng.try_leader_handler().unwrap().send_heartbeat(true);
        assert!(matches!(eng.output.take_commands().as_slice(), [
            Command::BroadcastHeartbeat {
                bypass_min_interval: true,
                ..
            }
        ]));
        eng.replication_handler().initiate_replication();
        assert!(!eng.output.take_commands().is_empty());
    }
}
