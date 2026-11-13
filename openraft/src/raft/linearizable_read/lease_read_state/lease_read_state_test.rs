use std::time::Duration;

use super::LeaseReadState;
use crate::Vote;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::ReadPolicy;
use crate::raft::linearizable_read::LinearizerOption;
use crate::raft::linearizable_read::ReadLogId;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::vote::raft_vote::RaftVoteExt;

const LEADER_LEASE: Duration = Duration::from_millis(100);

fn snapshot(acked_at: InstantOf<UTConfig>) -> LeaseReadState<UTConfig> {
    LeaseReadState {
        leader_vote: Vote::new_committed(2, 1).to_committed(),
        noop_log_id: log_id(2, 1, 3),
        committed: Some(log_id(2, 1, 5)),
        quorum_acked_at: Some(acked_at),
        leader_lease: LEADER_LEASE,
        self_quorum: false,
    }
}

#[test]
fn cached_acknowledgement_requires_strict_freshness() {
    let acked_at = UTConfig::<()>::now();
    let expires_at = acked_at + LEADER_LEASE;
    let one_tick = Duration::from_nanos(1);
    let cases = [
        (None, acked_at, false),
        (Some(acked_at), acked_at, true),
        (Some(acked_at), expires_at - one_tick, true),
        (Some(acked_at), expires_at, false),
        (Some(acked_at), expires_at + one_tick, false),
    ];
    let option = LinearizerOption::from(ReadPolicy::LeaseRead);
    for (acknowledged, now, want) in cases {
        let mut state = snapshot(acked_at);
        state.quorum_acked_at = acknowledged;
        let linearizer = state.try_linearizer(1, &option, now);
        let got = linearizer.is_some();
        assert_eq!(want, got, "acknowledged={acknowledged:?}, now={now:?}");
    }
}

#[test]
fn custom_acknowledgement_age_is_capped_at_leader_lease() {
    let acked_at = UTConfig::<()>::now();
    let shorter = Duration::from_millis(20);
    let longer = LEADER_LEASE * 2;
    let one_tick = Duration::from_nanos(1);
    let cases = [
        (shorter, shorter - one_tick, true),
        (shorter, shorter, false),
        (longer, LEADER_LEASE - one_tick, true),
        (longer, LEADER_LEASE, false),
    ];
    let state = snapshot(acked_at);
    for (age, elapsed, want) in cases {
        let option = LinearizerOption::new(Some(age), false);
        let now = acked_at + elapsed;
        let linearizer = state.try_linearizer(1, &option, now);
        let got = linearizer.is_some();
        assert_eq!(want, got, "age={age:?}, elapsed={elapsed:?}");
    }
}

#[test]
fn self_quorum_needs_no_acknowledgement() {
    let acked_at = UTConfig::<()>::now();
    let mut state = snapshot(acked_at);
    state.self_quorum = true;
    state.quorum_acked_at = None;
    let now = acked_at + LEADER_LEASE * 2;
    let option = LinearizerOption::from(ReadPolicy::LeaseRead);
    let linearizer = state.try_linearizer(1, &option, now);
    let linearizer = linearizer.expect("self quorum permits a cached lease read");
    let node_id = linearizer.node_id();
    assert_eq!(&1, node_id);
    let read_log_id = linearizer.read_log_id();
    let expected = ReadLogId::new(state.noop_log_id, state.committed);
    assert_eq!(expected, read_log_id);
    assert_eq!(&state.noop_log_id, linearizer.noop_log_id());
    assert_eq!(state.committed.as_ref(), linearizer.committed_log_id());
    let applied = linearizer.applied();
    assert_eq!(None, applied);
}

#[test]
fn zero_effective_age_keeps_self_quorum_on_the_core_path() {
    let now = UTConfig::<()>::now();
    let mut state = snapshot(now);
    state.self_quorum = true;
    let read_index = LinearizerOption::from(ReadPolicy::ReadIndex);
    let linearizer = state.try_linearizer(1, &read_index, now);
    let got = linearizer.is_some();
    assert!(!got, "ReadIndex must retain its core path");

    state.leader_lease = Duration::ZERO;
    let lease_read = LinearizerOption::from(ReadPolicy::LeaseRead);
    let linearizer = state.try_linearizer(1, &lease_read, now);
    let got = linearizer.is_some();
    assert!(!got, "a zero configured lease does not permit cached reads");
}

#[test]
fn linearizer_preserves_the_noop_boundary() {
    let now = UTConfig::<()>::now();
    let noop = log_id(2, 1, 3);
    let committed = log_id(2, 1, 5);
    let cases = [
        (None, noop),
        (Some(log_id(1, 1, 8)), noop),
        (Some(log_id(2, 1, 2)), noop),
        (Some(committed), committed),
    ];
    let option = LinearizerOption::from(ReadPolicy::LeaseRead);
    for (committed, expected_log_id) in cases {
        let mut state = snapshot(now);
        state.committed = committed;
        let linearizer = state.try_linearizer(1, &option, now);
        let linearizer = linearizer.expect("the acknowledgement is fresh");
        let read_log_id = linearizer.read_log_id();
        let expected = ReadLogId::from_log_id(expected_log_id);
        assert_eq!(expected, read_log_id);
        assert_eq!(&noop, linearizer.noop_log_id());
        assert_eq!(committed.as_ref(), linearizer.committed_log_id());
        let applied = linearizer.applied();
        assert_eq!(None, applied);
    }
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "snapshot no-op must belong to leader vote")]
fn snapshot_noop_must_belong_to_its_leader_vote() {
    let now = UTConfig::<()>::now();
    let mut state = snapshot(now);
    state.leader_vote = Vote::new_committed(3, 1).to_committed();
    let option = LinearizerOption::from(ReadPolicy::LeaseRead);
    let _linearizer = state.try_linearizer(1, &option, now);
}
