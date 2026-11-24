use std::collections::BTreeSet;
use std::panic::AssertUnwindSafe;
use std::panic::catch_unwind;
use std::time::Duration;

use maplit::btreeset;

use crate::Membership;
use crate::Vote;
use crate::base::shared_id_generator::SharedIdGenerator;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::proposer::Leader;
use crate::proposer::SendStamp;
use crate::type_config::TypeConfigExt;
use crate::vote::raft_vote::RaftVoteExt;

#[test]
fn test_new_tracks_write_voters_and_other_nodes() {
    let membership = Membership::new_with_defaults(vec![btreeset! {1,2}, btreeset! {2,3}], [4, 5]);
    let vote = Vote::new(2, 1).to_committed();
    let leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

    let progress_ids = leading.progress.iter().map(|entry| entry.id);
    let progress_ids = progress_ids.collect::<BTreeSet<_>>();
    assert_eq!(btreeset! {1,2,3,4,5}, progress_ids);
    let clock_ids = leading.clock_progress.iter().map(|entry| entry.id);
    let clock_ids = clock_ids.collect::<BTreeSet<_>>();
    assert_eq!(btreeset! {1,2,3,4,5}, clock_ids);
    let write_voters = (0..=6).filter(|id| leading.progress.is_voter(id) == Some(true)).collect::<Vec<_>>();
    assert_eq!(vec![1, 2, 3], write_voters);
    let clock_voters = (0..=6).filter(|id| leading.clock_progress.is_voter(id) == Some(true)).collect::<Vec<_>>();
    assert_eq!(vec![1, 2, 3], clock_voters);
}

#[test]
fn test_joint_clock_quorum_excludes_non_write_voters() {
    let membership = Membership::new_with_defaults(vec![btreeset! {1,2}, btreeset! {2,3}], [4]);
    let vote = Vote::new(2, 1).to_committed();
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());
    let now = UTConfig::<()>::now();
    let stamp = SendStamp::new(now, 0);

    let acked = leading.update_clock(&2, stamp);
    assert_eq!(None, acked);
    let acked = leading.update_clock(&4, stamp);
    assert_eq!(None, acked);
    let acked = leading.update_clock(&3, stamp);
    assert_eq!(Some(stamp), acked);
}

#[test]
fn test_leader_new_noop_log_id() {
    tracing::info!("--- vote greater than last log id, create new noop_log_id");
    {
        let vote = Vote::new(2, 2).to_committed();
        let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
        let leader = Leader::<UTConfig, _>::new(vote, membership, Some(log_id(1, 2, 3)), SharedIdGenerator::new());

        assert_eq!(leader.noop_log_id(), &log_id(2, 2, 4));
        assert_eq!(leader.last_log_id(), Some(&log_id(1, 2, 3)));
    }

    tracing::info!("--- no last log id, create new noop_log_id");
    {
        let vote = Vote::new(1, 2).to_committed();
        let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
        let leader = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

        assert_eq!(leader.noop_log_id(), &log_id(1, 2, 0));
        assert_eq!(leader.last_log_id(), None);
    }
}

/// A leader is never rebuilt with a leader id that its log already holds: it would assign the
/// log ids of entries lost in a crash to new entries.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "must be greater than the leader id of last_log_id")]
fn test_leader_new_rejects_the_leader_id_of_last_log_id() {
    let vote = Vote::new(1, 2).to_committed();
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
    let _ = Leader::<UTConfig, _>::new(vote, membership, Some(log_id(1, 2, 3)), SharedIdGenerator::new());
}

#[test]
fn test_leader_established() {
    let vote = Vote::new(2, 2).to_committed();
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
    let mut leader = Leader::<UTConfig, _>::new(vote, membership, Some(log_id(1, 2, 3)), SharedIdGenerator::new());

    let log_ids: Vec<_> = leader.assign_log_ids(1).unwrap().into_iter().collect();

    assert_eq!(
        log_ids,
        vec![log_id(2, 2, 4)],
        "entry log id assigned following last-log-id"
    );
    assert_eq!(Some(log_id(2, 2, 4)), leader.last_log_id);
}

#[test]
fn test_1_entry_none_last_log_id() {
    let vote = Vote::new(0, 0).to_committed();
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

    let log_ids: Vec<_> = leading.assign_log_ids(1).unwrap().into_iter().collect();

    assert_eq!(log_ids, vec![log_id(0, 0, 0)]);
    assert_eq!(Some(log_id(0, 0, 0)), leading.last_log_id);
}

#[test]
fn test_no_entries_provided() {
    let vote = Vote::new(2, 2).to_committed();
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, Some(log_id(1, 1, 8)), SharedIdGenerator::new());

    let log_ids = leading.assign_log_ids(0);
    assert_eq!(log_ids, None);
    assert_eq!(Some(log_id(1, 1, 8)), leading.last_log_id);
}

#[test]
fn test_multiple_entries() {
    let vote = Vote::new(2, 2).to_committed();
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, Some(log_id(1, 1, 8)), SharedIdGenerator::new());

    let log_ids: Vec<_> = leading.assign_log_ids(3).unwrap().into_iter().collect();
    assert_eq!(log_ids, vec![log_id(2, 2, 9), log_id(2, 2, 10), log_id(2, 2, 11)]);
    assert_eq!(Some(log_id(2, 2, 11)), leading.last_log_id);
}

/// The last batch a leader can assign ends at index `u64::MAX - 1`, so that iterating it can end at
/// the index after its last one. After that batch, only an empty batch is accepted.
#[test]
fn test_assign_log_ids_up_to_the_last_assignable_index() {
    let vote = Vote::new(2, 2).to_committed();
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
    let last_log_id = log_id(1, 1, u64::MAX - 3);
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, Some(last_log_id), SharedIdGenerator::new());

    tracing::info!("--- assign the last 2 indexes, up to u64::MAX - 1");
    {
        let log_ids: Vec<_> = leading.assign_log_ids(2).unwrap().into_iter().collect();
        assert_eq!(log_ids, vec![log_id(2, 2, u64::MAX - 2), log_id(2, 2, u64::MAX - 1)]);
        assert_eq!(Some(log_id(2, 2, u64::MAX - 1)), leading.last_log_id);
    }

    tracing::info!("--- index u64::MAX is not assignable: reject 1 more entry");
    {
        let payload = catch_unwind(AssertUnwindSafe(|| leading.assign_log_ids(1))).unwrap_err();
        let msg = payload.downcast::<String>().unwrap();
        assert!(msg.contains("the last assignable log index is u64::MAX - 1"), "{msg}");
        assert_eq!(Some(log_id(2, 2, u64::MAX - 1)), leading.last_log_id);
    }

    tracing::info!("--- an empty batch needs no index");
    {
        assert_eq!(None, leading.assign_log_ids(0));
        assert_eq!(Some(log_id(2, 2, u64::MAX - 1)), leading.last_log_id);
    }
}

/// A batch larger than the indexes left is rejected in every build, before the last log id
/// changes. Unchecked, a build without overflow checks would store a wrapped last log id.
#[test]
fn test_assign_log_ids_rejects_a_batch_beyond_the_last_assignable_index() {
    let vote = Vote::new(2, 2).to_committed();
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
    let last_log_id = log_id(1, 1, u64::MAX - 3);
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, Some(last_log_id), SharedIdGenerator::new());

    for count in [3, usize::MAX] {
        tracing::info!(count, "--- 2 indexes left from u64::MAX - 2: reject the batch");
        {
            let payload = catch_unwind(AssertUnwindSafe(|| leading.assign_log_ids(count))).unwrap_err();
            let msg = payload.downcast::<String>().unwrap();
            assert!(msg.contains("the last assignable log index is u64::MAX - 1"), "{msg}");
            assert_eq!(Some(log_id(1, 1, u64::MAX - 3)), leading.last_log_id);
        }
    }
}

#[test]
fn test_leading_last_quorum_acked_time_leader_is_voter() {
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
    let vote = Vote::new(2, 1).to_committed();
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

    let now1 = UTConfig::<()>::now();

    leading.update_clock(&2, SendStamp::new(now1, 0));
    let t1 = leading.last_quorum_acked_time();
    assert_eq!(Some(now1), t1, "n1(leader) and n2 acked, t1 > t2");
}

#[test]
fn test_leading_last_quorum_acked_time_leader_is_learner() {
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
    let vote = Vote::new(2, 4).to_committed();
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

    let t2 = UTConfig::<()>::now();
    leading.update_clock(&2, SendStamp::new(t2, 0));
    let t = leading.last_quorum_acked_time();
    assert!(t.is_none(), "n1(leader+learner) does not count in quorum");

    let t3 = UTConfig::<()>::now();
    leading.update_clock(&3, SendStamp::new(t3, 0));
    let t = leading.last_quorum_acked_time();
    assert_eq!(Some(t2), t, "n2 and n3 acked");
}

#[test]
fn test_leading_last_quorum_acked_time_leader_is_not_member() {
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
    let vote = Vote::new(2, 5).to_committed();
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

    let t2 = UTConfig::<()>::now();
    leading.update_clock(&2, SendStamp::new(t2, 0));
    let t = leading.last_quorum_acked_time();
    assert!(t.is_none(), "n1(leader+learner) does not count in quorum");

    let t3 = UTConfig::<()>::now();
    leading.update_clock(&3, SendStamp::new(t3, 0));
    let t = leading.last_quorum_acked_time();
    assert_eq!(Some(t2), t, "n2 and n3 acked");
}

#[test]
fn test_clock_quorum_not_enough() {
    // Voters {1,2,3} with learner {4}; node 1 is the leader.
    let membership = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
    let cluster = "[{1, 2, 3}]";
    let vote = Vote::new(2, 1).to_committed();
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

    let now = UTConfig::<()>::now();
    let t1 = SendStamp::new(now, 0);
    let t2 = SendStamp::new(now + Duration::from_millis(10), 0);

    let err = leading.clock_quorum_not_enough(t1);
    assert_eq!(
        btreeset! {1},
        err.got,
        "the leader grants its own RPCs before any response"
    );
    assert_eq!(cluster, err.cluster);

    leading.update_clock(&2, t1);
    let err = leading.clock_quorum_not_enough(t1);
    assert_eq!(btreeset! {1}, err.got, "an RPC sent at t1 is not newer than t1");

    leading.update_clock(&2, t2);
    let err = leading.clock_quorum_not_enough(t1);
    assert_eq!(btreeset! {1, 2}, err.got, "n2 acked an RPC sent after t1");

    leading.update_clock(&4, t2);
    let err = leading.clock_quorum_not_enough(t2);
    assert_eq!(
        btreeset! {1},
        err.got,
        "learner n4 is not counted, and n2's ack is too old"
    );
}

/// The clock may not move between a read and the heartbeat it triggers, as on a simulated
/// runtime. The heartbeat round still tells an RPC sent before the read from one sent after it.
#[test]
fn test_clock_quorum_orders_same_time_by_heartbeat_round() {
    // Voters {1,2,3} with learner {4}; node 1 is the leader.
    let membership = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
    let vote = Vote::new(2, 1).to_committed();
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());
    let now = UTConfig::<()>::now();

    tracing::info!("--- n2 acks a replication RPC sent at `now`, before the read arrives");
    leading.update_clock(&2, SendStamp::new(now, 0));

    let round_at_read = leading.heartbeat_round();
    let read_threshold = SendStamp::new(now, round_at_read);

    tracing::info!(round_at_read, "--- an RPC sent before the read does not confirm it");
    {
        let err = leading.clock_quorum_not_enough(read_threshold);
        assert_eq!(btreeset! {1}, err.got);
    }

    tracing::info!("--- n3 acks the heartbeat that the read triggers, still at `now`");
    {
        let round = leading.next_heartbeat_round();
        let heartbeat = SendStamp::new(now, round);
        leading.update_clock(&3, heartbeat);

        let err = leading.clock_quorum_not_enough(read_threshold);
        assert_eq!(btreeset! {1, 3}, err.got);

        let quorum_acked = leading.last_quorum_acked_stamp();
        assert_eq!(Some(heartbeat), quorum_acked);
    }
}

#[test]
fn test_need_heartbeat() {
    let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
    let vote = Vote::new(2, 1).to_committed();
    let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

    let min_interval = Duration::from_millis(100);
    let t0 = UTConfig::<()>::now();

    assert!(
        leading.need_heartbeat(&2, t0, min_interval),
        "never acknowledged: must send"
    );
    assert!(
        leading.need_heartbeat(&9, t0, min_interval),
        "unknown target: must send"
    );

    leading.clock_progress.increase_to(&2, Some(SendStamp::new(t0, 0)));

    assert!(
        !leading.need_heartbeat(&2, t0 + Duration::from_millis(99), min_interval),
        "acked RPC sent within min_interval: suppressed"
    );
    assert!(
        leading.need_heartbeat(&2, t0 + Duration::from_millis(100), min_interval),
        "acked RPC sending time is min_interval old: must send"
    );
    assert!(
        leading.need_heartbeat(&2, t0, Duration::ZERO),
        "zero min_interval disables suppression"
    );
    assert!(
        leading.need_heartbeat(&3, t0 + Duration::from_millis(99), min_interval),
        "only the acked follower is suppressed"
    );
}
