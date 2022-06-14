use std::sync::Arc;

use maplit::btreeset;

use crate::engine::Command;
use crate::engine::Engine;
use crate::DummyNetwork;
use crate::DummyStorage;
use crate::EffectiveMembership;
use crate::Entry;
use crate::EntryPayload;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;
use crate::Vote;

crate::declare_raft_types!(
    pub(crate) Foo: D=(), R=(), S = DummyStorage<Self>, N = DummyNetwork<Self>, NodeId=u64
);

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn blank(term: u64, index: u64) -> Entry<Foo> {
    Entry {
        log_id: log_id(term, index),
        payload: EntryPayload::<Foo>::Blank,
    }
}

fn m01() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {0,1}], None)
}

fn m2() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {2}], None)
}

/// members: {2}, learners: {1}
fn m2_1() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {2}], Some(btreeset! {1}))
}

fn m23() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {2,3}], None)
}

fn m34() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {3,4}], None)
}

fn eng() -> Engine<u64> {
    let mut eng = Engine::<u64> {
        id: 2, // make it a member
        single_node_cluster: btreeset! {2},
        ..Default::default()
    };
    eng.state.vote = Vote::new(3, 2);
    eng.state.log_ids.append(log_id(1, 1));
    eng.state.log_ids.append(log_id(2, 3));
    eng.state.last_log_id = Some(log_id(2, 3));
    eng.state.membership_state.committed = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()));
    eng
}

#[test]
fn test_leader_append_entries_empty() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.leader_append_entries(&mut Vec::<Entry<Foo>>::new());

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(log_id(2, 3)), eng.state.last_log_id);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: false
        },
        eng.metrics_flags
    );

    assert_eq!(0, eng.commands.len());

    Ok(())
}

#[test]
fn test_leader_append_entries_normal() -> anyhow::Result<()> {
    let mut eng = eng();

    // log id will be assigned by eng.
    eng.leader_append_entries(&mut [
        blank(1, 1), //
        blank(1, 1),
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            LogId::new(LeaderId::new(3, 2), 4),
            LogId::new(LeaderId::new(3, 2), 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 2), 6)), eng.state.last_log_id);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            Command::ReplicateInputEntries { range: 0..3 },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_leader_append_entries_fast_commit() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m2()));

    // log id will be assigned by eng.
    eng.leader_append_entries(&mut [
        blank(1, 1), //
        blank(1, 1),
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            LogId::new(LeaderId::new(3, 2), 4),
            LogId::new(LeaderId::new(3, 2), 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 2), 6)), eng.state.last_log_id);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m2()))
        },
        eng.state.membership_state
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 2), 6)), eng.state.committed);

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            Command::LeaderCommit {
                upto: LogId::new(LeaderId::new(3, 2), 6)
            },
            Command::ReplicateInputEntries { range: 0..3 },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.commands
    );

    Ok(())
}

/// With membership log, fast-commit is now allowed.
#[test]
fn test_leader_append_entries_with_membership() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m2()));

    // log id will be assigned by eng.
    eng.leader_append_entries(&mut [
        blank(1, 1), //
        Entry {
            log_id: log_id(1, 1),
            payload: EntryPayload::Membership(m34()),
        },
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            LogId::new(LeaderId::new(3, 2), 4),
            LogId::new(LeaderId::new(3, 2), 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 2), 6)), eng.state.last_log_id);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(3, 2), 5)),
                m34()
            ))
        },
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(
                    Some(LogId::new(LeaderId::new(3, 2), 5)),
                    m34()
                )),
            },
            Command::ReplicateInputEntries { range: 0..3 },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.commands
    );

    Ok(())
}

/// With membership log, fast-commit is now allowed.
#[test]
fn test_leader_append_entries_allow_fast_commit_if_no_member_changed() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m2()));

    // log id will be assigned by eng.
    eng.leader_append_entries(&mut [
        blank(1, 1), //
        Entry {
            log_id: log_id(1, 1),
            payload: EntryPayload::Membership(m2_1()),
        },
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            LogId::new(LeaderId::new(3, 2), 4),
            LogId::new(LeaderId::new(3, 2), 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 2), 6)), eng.state.last_log_id);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(3, 2), 5)),
                m2_1()
            ))
        },
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(
                    Some(LogId::new(LeaderId::new(3, 2), 5)),
                    m2_1()
                )),
            },
            Command::LeaderCommit {
                upto: LogId::new(LeaderId::new(3, 2), 6)
            },
            Command::ReplicateInputEntries { range: 0..3 },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.commands
    );

    Ok(())
}
