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
fn inactive_and_probe_deadlines() {
    let mut leader = Leader::<UTConfig, _>::new(
        Vote::new(1, 0).to_committed(),
        vec![btreeset! {0, 1, 2}],
        [],
        None,
        SharedIdGenerator::new(),
    );
    let start = UTConfig::<()>::now();
    let window = Duration::from_millis(100);
    let interval = Duration::from_millis(300);

    tracing::info!("Allow a new Leader one evidence window before closing admission");
    {
        leader.observe_quorum(start, window);
        assert!(!leader.check_activity(start + window - Duration::from_millis(1), window, interval));
        assert_eq!(
            leader.activity,
            Some(LeaderActivity::Active {
                next_check_at: start + window
            })
        );

        let closed_at = start + window;
        assert!(leader.check_activity(closed_at, window, interval));
        assert_eq!(
            leader.activity,
            Some(LeaderActivity::Inactive {
                next_probe_at: closed_at + interval
            })
        );
    }

    tracing::info!("Schedule a probe from the actual submission time");
    {
        let due = start + window + interval;
        assert!(!leader.is_quorum_probe_due(due - Duration::from_millis(1)));
        assert!(leader.is_quorum_probe_due(due));

        let submitted_at = due + Duration::from_millis(50);
        leader.schedule_next_quorum_probe(submitted_at, interval);
        assert!(!leader.is_quorum_probe_due(submitted_at + interval - Duration::from_millis(1)));
        assert!(leader.is_quorum_probe_due(submitted_at + interval));
    }
}

#[test]
fn fresh_quorum_refreshes_deadline_and_reopens_admission() {
    let mut leader = Leader::<UTConfig, _>::new(
        Vote::new(1, 0).to_committed(),
        vec![btreeset! {0, 1, 2}],
        [],
        None,
        SharedIdGenerator::new(),
    );
    let start = UTConfig::<()>::now();
    let window = Duration::from_millis(100);
    let interval = Duration::from_millis(300);
    leader.observe_quorum(start, window);

    tracing::info!("Fresh evidence extends but never shortens an active deadline");
    {
        let old_ack = start - Duration::from_millis(10);
        leader.update_clock(&1, old_ack);
        assert!(!leader.refresh_activity(start, window));
        assert_eq!(
            leader.activity,
            Some(LeaderActivity::Active {
                next_check_at: start + window
            })
        );

        let acked_at = start + Duration::from_millis(50);
        leader.update_clock(&1, acked_at);
        let received_at = acked_at + Duration::from_millis(20);
        assert!(!leader.refresh_activity(received_at, window));
        assert_eq!(
            leader.activity,
            Some(LeaderActivity::Active {
                next_check_at: acked_at + window
            })
        );
        assert!(leader.is_lease_valid_at(acked_at + window - Duration::from_millis(1), window));
        assert!(!leader.is_lease_valid_at(acked_at + window, window));
    }

    tracing::info!("Expired evidence cannot reopen admission, but fresh evidence does");
    {
        let expired_at = start + Duration::from_millis(150);
        assert!(leader.check_activity(expired_at, window, interval));
        assert!(!leader.refresh_activity(expired_at, window));

        let acked_at = expired_at + Duration::from_millis(10);
        leader.update_clock(&1, acked_at);
        assert!(leader.refresh_activity(acked_at, window));
        assert_eq!(
            leader.activity,
            Some(LeaderActivity::Active {
                next_check_at: acked_at + window
            })
        );
    }
}

#[test]
fn insufficient_quorum_does_not_extend_active_deadline() {
    let mut leader = Leader::<UTConfig, _>::new(
        Vote::new(1, 0).to_committed(),
        vec![btreeset! {0, 1, 2, 3, 4}],
        [],
        None,
        SharedIdGenerator::new(),
    );
    let start = UTConfig::<()>::now();
    let window = Duration::from_millis(100);
    leader.observe_quorum(start, window);

    tracing::info!("Repeated evidence from one follower does not form a voter quorum");
    {
        let deadline = start + window;

        for acked_at in [start + Duration::from_millis(25), start + Duration::from_millis(50)] {
            leader.update_clock(&1, acked_at);
            assert_eq!(None, leader.last_quorum_acked_time());
            assert!(!leader.refresh_activity(acked_at, window));
            assert_eq!(
                leader.activity,
                Some(LeaderActivity::Active {
                    next_check_at: deadline
                })
            );
        }
    }
}

#[test]
fn self_quorum_refreshes_from_now() {
    let mut leader = Leader::<UTConfig, _>::new(
        Vote::new(1, 0).to_committed(),
        vec![btreeset! {0}],
        [],
        None,
        SharedIdGenerator::new(),
    );
    let start = UTConfig::<()>::now();
    let window = Duration::from_millis(100);
    let interval = Duration::from_millis(300);
    leader.observe_quorum(start, window);

    tracing::info!("A self quorum remains active and schedules the next check from now");
    {
        let now = start + window;
        assert!(!leader.check_activity(now, window, interval));
        assert_eq!(
            leader.activity,
            Some(LeaderActivity::Active {
                next_check_at: now + window
            })
        );
    }
}
