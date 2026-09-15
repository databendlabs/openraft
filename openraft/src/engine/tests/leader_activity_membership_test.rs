use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use super::leader_activity_test::engine;
use crate::Membership;
use crate::MembershipState;
use crate::engine::Command;
use crate::engine::testing::UTConfig;
use crate::proposer::leader_activity::LeaderActivity;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;

#[test]
fn only_voter_changes_restart_the_active_observation_window() {
    let mut eng = engine(true);
    let now = UTConfig::<()>::now();
    let deadline = now + Duration::from_millis(20);
    eng.leader.as_mut().unwrap().activity = Some(LeaderActivity::AwaitingQuorum { inactive_at: deadline });

    tracing::info!("Learner-only changes preserve the existing grace deadline and voter evidence");
    {
        eng.leader.as_mut().unwrap().update_clock(&1, now);
        let membership = Membership::new_with_defaults(vec![btreeset! {0, 1, 2, 3, 4}], [5, 6]);
        let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership));
        eng.state.membership_state = MembershipState::new(stored.clone(), stored);
        eng.replication_handler().rebuild_progresses();
        assert_eq!(
            eng.leader.as_ref().unwrap().activity,
            Some(LeaderActivity::AwaitingQuorum { inactive_at: deadline })
        );
        assert_eq!(
            eng.leader.as_ref().unwrap().clock_progress.try_get(&1).unwrap().val,
            Some(now)
        );
        assert_eq!(
            eng.leader.as_ref().unwrap().clock_progress.try_get(&6).unwrap().val,
            None
        );
    }

    tracing::info!("A voter quorum change cancels grace and grants a complete new evidence window");
    {
        let membership = Membership::new_with_defaults(vec![btreeset! {0, 1, 2, 3, 4, 6}], [5]);
        let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership));
        eng.state.membership_state = MembershipState::new(stored.clone(), stored);
        let before = UTConfig::<()>::now();
        eng.replication_handler().rebuild_progresses();
        let after = UTConfig::<()>::now();
        let Some(LeaderActivity::Active {
            observe_until: Some(observe_until),
        }) = eng.leader.as_ref().unwrap().activity
        else {
            panic!("Voter change must restart observation");
        };
        let window = eng.config.timer_config.leader_lease;
        assert!(before + window <= observe_until && observe_until <= after + window);
        assert!(eng.output.take_commands().is_empty());
    }
}

#[test]
fn inactive_membership_change_only_recovers_with_a_fresh_complete_quorum() {
    let mut eng = engine(true);
    let now = UTConfig::<()>::now();
    let deadline = now + Duration::from_millis(700);
    eng.leader.as_mut().unwrap().activity = Some(LeaderActivity::Inactive {
        next_probe_at: deadline,
    });
    eng.leader.as_mut().unwrap().update_clock(&1, now + Duration::from_secs(1));

    tracing::info!("An unsatisfied new voter quorum leaves admission and the probe deadline unchanged");
    {
        let membership = Membership::new_with_defaults(vec![btreeset! {0, 1, 2, 3, 6}], [4, 5]);
        let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership));
        eng.state.membership_state = MembershipState::new(stored.clone(), stored);
        eng.replication_handler().rebuild_progresses();
        assert_eq!(
            eng.leader.as_ref().unwrap().activity,
            Some(LeaderActivity::Inactive {
                next_probe_at: deadline
            })
        );
        assert!(eng.output.take_commands().is_empty());
    }

    tracing::info!("Existing fresh evidence satisfying the changed quorum opens admission immediately");
    {
        let membership = Membership::new_with_defaults(vec![btreeset! {0, 1, 2}], [3, 4, 5, 6]);
        let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership));
        eng.state.membership_state = MembershipState::new(stored.clone(), stored);
        eng.replication_handler().rebuild_progresses();
        assert_eq!(
            eng.leader.as_ref().unwrap().activity,
            Some(LeaderActivity::Active { observe_until: None })
        );
        assert!(matches!(eng.output.take_commands().as_slice(), [
            Command::SetLeaderActivity { active: true, .. }
        ]));
    }
}
