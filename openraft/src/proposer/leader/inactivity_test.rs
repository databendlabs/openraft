use std::time::Duration;

use maplit::btreeset;

use crate::Vote;
use crate::base::shared_id_generator::SharedIdGenerator;
use crate::engine::testing::UTConfig;
use crate::proposer::Leader;
use crate::proposer::leader_activity::LeaderActivity;
use crate::type_config::TypeConfigExt;
use crate::vote::raft_vote::RaftVoteExt;

#[test]
fn observation_grace_and_probe_deadlines() {
    let mut leader = Leader::<UTConfig, _>::new(
        Vote::new(1, 0).to_committed(),
        vec![btreeset! {0, 1, 2}],
        [],
        None,
        SharedIdGenerator::new(),
    );
    let start = UTConfig::<()>::now();
    let window = Duration::from_millis(100);
    let grace = Duration::from_millis(20);
    let interval = Duration::from_millis(300);

    tracing::info!("Allow a new Leader a full evidence window before starting grace");
    {
        leader.observe_quorum(start, window);
        assert!(!leader.check_activity(start + window - Duration::from_millis(1), window, grace, interval));
        assert_eq!(
            leader.activity,
            Some(LeaderActivity::Active {
                observe_until: Some(start + window)
            })
        );
        assert!(!leader.check_activity(start + window, window, grace, interval));
        assert_eq!(
            leader.activity,
            Some(LeaderActivity::AwaitingQuorum {
                inactive_at: start + window + grace
            })
        );
        assert!(!leader.check_activity(
            start + window + grace - Duration::from_millis(1),
            window,
            grace,
            interval
        ));
        assert_eq!(
            leader.activity,
            Some(LeaderActivity::AwaitingQuorum {
                inactive_at: start + window + grace
            })
        );
    }

    tracing::info!("Close only at the fixed grace deadline and anchor R to actual closure");
    {
        let due = start + window + grace;
        assert!(leader.check_activity(due, window, grace, interval));
        let closed_at = due + Duration::from_millis(50);
        leader.arm_quorum_probe(closed_at, interval);
        assert!(!leader.is_quorum_probe_due(closed_at + interval - Duration::from_millis(1)));
        assert!(leader.is_quorum_probe_due(closed_at + interval));
        assert!(!leader.check_activity(closed_at + interval, window, grace, interval));
        assert!(
            leader.is_quorum_probe_due(closed_at + interval),
            "An unsent probe must remain due"
        );
        let submitted_at = closed_at + interval + Duration::from_millis(50);
        leader.arm_quorum_probe(submitted_at, interval);
        assert!(!leader.is_quorum_probe_due(submitted_at + interval - Duration::from_millis(1)));
        assert!(leader.is_quorum_probe_due(submitted_at + interval));
    }
}

#[test]
fn fresh_quorum_cancels_grace_and_reopens_inactive() {
    let mut leader = Leader::<UTConfig, _>::new(
        Vote::new(1, 0).to_committed(),
        vec![btreeset! {0, 1, 2, 3, 4}],
        [5],
        None,
        SharedIdGenerator::new(),
    );
    let start = UTConfig::<()>::now();
    let window = Duration::from_millis(100);
    let grace = Duration::from_millis(20);
    let interval = Duration::from_millis(300);
    leader.observe_quorum(start, window);
    let due = start + window + grace;
    leader.check_activity(start + window, window, grace, interval);

    tracing::info!("Revalidate fresh quorum at the exact grace deadline before closing");
    {
        leader.update_clock(&1, due);
        leader.update_clock(&2, due);
        assert!(!leader.check_activity(due, window, grace, interval));
        assert_eq!(leader.activity, Some(LeaderActivity::Active { observe_until: None }));
        assert!(leader.is_lease_valid_at(due + window - Duration::from_millis(1), window));
        assert!(!leader.is_lease_valid_at(due + window, window));
        assert!(!leader.check_activity(due + window, window, grace, interval));
        assert!(leader.check_activity(due + window + grace, window, grace, interval));
    }

    tracing::info!("One bridge or a learner cannot reopen; expired timestamps remain historical");
    {
        let now = due + window + grace;
        let deadline = leader.activity;
        leader.update_clock(&1, now);
        leader.update_clock(&5, now);
        assert!(!leader.try_recover_activity(now, window));
        leader.update_clock(&2, now - window);
        assert!(!leader.try_recover_activity(now, window));
        assert_eq!(leader.activity, deadline);
        leader.update_clock(&2, now);
        assert!(leader.try_recover_activity(now, window));
        assert_eq!(leader.activity, Some(LeaderActivity::Active { observe_until: None }));
        assert!(!leader.is_quorum_probe_due(now + interval));
    }
}

#[test]
fn zero_grace_and_joint_quorum() {
    let mut leader = Leader::<UTConfig, _>::new(
        Vote::new(1, 0).to_committed(),
        vec![btreeset! {0, 1, 2}, btreeset! {2, 3, 4}],
        [],
        None,
        SharedIdGenerator::new(),
    );
    let start = UTConfig::<()>::now();
    let window = Duration::from_millis(100);
    let interval = Duration::from_millis(300);
    leader.observe_quorum(start, window);
    let now = start + window;

    tracing::info!("Zero grace closes in the first failed tick after observation");
    {
        assert!(leader.check_activity(now, window, Duration::ZERO, interval));
    }

    tracing::info!("Recovery must satisfy both configurations through the existing quorum abstraction");
    {
        leader.update_clock(&1, now);
        assert!(!leader.try_recover_activity(now, window));
        leader.update_clock(&2, now);
        assert!(!leader.try_recover_activity(now, window));
        leader.update_clock(&3, now);
        assert!(leader.try_recover_activity(now, window));
        assert_eq!(leader.activity, Some(LeaderActivity::Active { observe_until: None }));
    }
}
