use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use super::leader_activity_test::engine;
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

#[test]
fn normal_election_initializes_activity_without_seeding_remote_clocks() {
    for enabled in [false, true] {
        let mut eng = Engine::<UTConfig>::testing_default(0);
        eng.state.enable_validation(false);
        let membership = Membership::new_with_defaults(vec![btreeset! {0, 1, 2}], []);
        let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(0, 0, 0)), membership));
        eng.state.membership_state = MembershipState::new(stored.clone(), stored);
        eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::ZERO, Vote::new(0, 0));
        eng.state.log_ids.append(log_id(0, 0, 0));
        eng.state.server_state = ServerState::Follower;
        let grace = Duration::from_millis(20);
        eng.config.quorum_loss_grace = enabled.then_some(grace);
        eng.config.quorum_loss_probe_interval = Some(Duration::from_millis(700));

        tracing::info!(
            enabled,
            "Elect through the real Candidate-to-Leader path without changing membership"
        );
        let (before, after) = {
            let before = UTConfig::<()>::now();
            eng.elect();
            let vote = *eng.candidate_ref().unwrap().vote_ref();
            eng.handle_vote_resp(0, VoteResponse::new(vote, Some(log_id(0, 0, 0)), true));
            eng.handle_vote_resp(1, VoteResponse::new(vote, Some(log_id(0, 0, 0)), true));
            let after = UTConfig::<()>::now();
            assert_eq!(ServerState::Leader, eng.state.server_state);
            assert_eq!(&Vote::new_committed(1, 0), eng.state.vote_ref());
            eng.output.take_commands();
            (before, after)
        };

        tracing::info!(
            enabled,
            "Initialize only the observation window, not remote ACK evidence"
        );
        let (initial_activity, observe_until) = {
            let leader = eng.leader.as_ref().unwrap();
            assert!(leader.clock_progress.iter().filter(|entry| entry.id != 0).all(|entry| entry.val.is_none()));
            let window = eng.config.timer_config.leader_lease;
            let deadline = if enabled {
                let Some(LeaderActivity::Active {
                    observe_until: Some(deadline),
                }) = leader.activity
                else {
                    panic!("Enabled elected Leader must observe quorum");
                };
                assert!(before + window <= deadline && deadline <= after + window);
                deadline
            } else {
                assert_eq!(None, leader.activity);
                after + window
            };
            (leader.activity, deadline)
        };

        tracing::info!(
            enabled,
            "Keep normal traffic through W, then close after G without quorum evidence"
        );
        {
            eng.handle_leader_activity(observe_until - Duration::from_millis(1));
            assert_eq!(initial_activity, eng.leader.as_ref().unwrap().activity);
            assert!(eng.output.take_commands().is_empty());
            eng.handle_leader_activity(observe_until);
            let inactive_at = observe_until + grace;
            let expected = enabled.then_some(LeaderActivity::AwaitingQuorum { inactive_at });
            assert_eq!(expected, eng.leader.as_ref().unwrap().activity);
            assert!(eng.output.take_commands().is_empty());

            eng.handle_leader_activity(inactive_at);
            if enabled {
                assert!(eng.leader.as_ref().unwrap().is_inactive());
                assert!(matches!(eng.output.take_commands().as_slice(), [
                    Command::SetLeaderActivity { active: false, .. }
                ]));
            } else {
                assert_eq!(None, eng.leader.as_ref().unwrap().activity);
                assert!(eng.output.take_commands().is_empty());
            }
            assert_eq!(ServerState::Leader, eng.state.server_state);
            assert_eq!(&Vote::new_committed(1, 0), eng.state.vote_ref());
        }
    }
}

#[test]
fn awaiting_response_can_cancel_elapsed_grace_without_opening_traffic() {
    let mut eng = engine(true);
    let now = UTConfig::<()>::now();
    eng.leader.as_mut().unwrap().activity = Some(LeaderActivity::AwaitingQuorum {
        inactive_at: now - Duration::from_millis(1),
    });

    tracing::info!("A partial fresh voter set leaves the elapsed grace deadline unchanged");
    {
        let stream = eng.leader.as_ref().unwrap().progress.try_get(&1).unwrap().data.stream_id;
        eng.replication_handler().try_update_leader_clock(stream, 1, now);
        assert_eq!(
            eng.leader.as_ref().unwrap().activity,
            Some(LeaderActivity::AwaitingQuorum {
                inactive_at: now - Duration::from_millis(1),
            })
        );
    }

    tracing::info!("Fresh quorum processed before the due tick cancels grace without an activity command");
    {
        let stream = eng.leader.as_ref().unwrap().progress.try_get(&2).unwrap().data.stream_id;
        eng.replication_handler().try_update_leader_clock(stream, 2, now);
        assert_eq!(
            eng.leader.as_ref().unwrap().activity,
            Some(LeaderActivity::Active { observe_until: None })
        );
        assert!(
            eng.output.take_commands().is_empty(),
            "Awaiting quorum never closed traffic admission"
        );
    }
}
