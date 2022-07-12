use std::sync::Arc;

use maplit::btreeset;
#[allow(unused_imports)] use pretty_assertions::assert_eq;
#[allow(unused_imports)] use pretty_assertions::assert_ne;
#[allow(unused_imports)] use pretty_assertions::assert_str_eq;

use crate::engine::Command;
use crate::engine::Engine;
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
    pub(crate) Foo: D=(), R=(), NodeId=u64
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

fn m1() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {1}], None)
}

/// members: {1}, learners: {2}
fn m1_2() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {1}], Some(btreeset! {2}))
}

fn m13() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {1,3}], None)
}

fn m23() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {2,3}], None)
}

fn m34() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {3,4}], None)
}

fn eng() -> Engine<u64> {
    let mut eng = Engine::<u64> {
        id: 1, // make it a member
        ..Default::default()
    };
    eng.state.last_applied = Some(log_id(0, 0));
    eng.state.vote = Vote::new_committed(3, 1);
    eng.state.log_ids.append(log_id(1, 1));
    eng.state.log_ids.append(log_id(2, 3));
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
    assert_eq!(Some(log_id(2, 3)), eng.state.last_log_id());
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
            LogId::new(LeaderId::new(3, 1), 4),
            LogId::new(LeaderId::new(3, 1), 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 1), 6)), eng.state.last_log_id());
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
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1()));
    eng.state.new_leader();

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
            LogId::new(LeaderId::new(3, 1), 4),
            LogId::new(LeaderId::new(3, 1), 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 1), 6)), eng.state.last_log_id());
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1())),
        },
        eng.state.membership_state
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 1), 6)), eng.state.committed);

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
            Command::ReplicateCommitted {
                committed: Some(log_id(3, 6))
            },
            Command::LeaderCommit {
                since: None,
                upto: LogId::new(LeaderId::new(3, 1), 6)
            },
            Command::ReplicateInputEntries { range: 0..3 },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.commands
    );

    Ok(())
}

/// With membership log, fast-commit upto membership entry that is not a single-node cluster.
/// Leader is no longer a voter should work.
#[test]
fn test_leader_append_entries_fast_commit_upto_membership_entry() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1()));
    eng.state.new_leader();

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
            LogId::new(LeaderId::new(3, 1), 4),
            LogId::new(LeaderId::new(3, 1), 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 1), 6)), eng.state.last_log_id());
    assert_eq!(
        MembershipState {
            // previous effective become committed.
            committed: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1())),
            // new effective.
            effective: Arc::new(EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(3, 1), 5)),
                m34()
            ))
        },
        eng.state.membership_state
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 1), 4)), eng.state.committed);

    assert_eq!(
        MetricsChangeFlags {
            leader: true,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            Command::ReplicateCommitted {
                committed: Some(log_id(3, 4))
            },
            Command::LeaderCommit {
                since: None,
                upto: LogId::new(LeaderId::new(3, 1), 4)
            },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(
                    Some(LogId::new(LeaderId::new(3, 1), 5)),
                    m34()
                )),
            },
            Command::UpdateReplicationStreams {
                remove: vec![],
                add: vec![(3, None), (4, None)]
            },
            Command::ReplicateInputEntries { range: 0..3 },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.commands
    );

    Ok(())
}

/// With membership log, fast-commit is allowed if no voter change.
#[test]
fn test_leader_append_entries_fast_commit_membership_no_voter_change() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1()));
    eng.state.new_leader();

    // log id will be assigned by eng.
    eng.leader_append_entries(&mut [
        blank(1, 1), //
        Entry {
            log_id: log_id(1, 1),
            payload: EntryPayload::Membership(m1_2()),
        },
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            LogId::new(LeaderId::new(3, 1), 4),
            LogId::new(LeaderId::new(3, 1), 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 1), 6)), eng.state.last_log_id());
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(3, 1), 5)),
                m1_2()
            )),
            effective: Arc::new(EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(3, 1), 5)),
                m1_2()
            ))
        },
        eng.state.membership_state
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 1), 6)), eng.state.committed);

    assert_eq!(
        MetricsChangeFlags {
            leader: true,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            // first commit upto the membership entry(exclusive).
            Command::ReplicateCommitted {
                committed: Some(log_id(3, 4))
            },
            Command::LeaderCommit {
                since: None,
                upto: LogId::new(LeaderId::new(3, 1), 4)
            },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(
                    Some(LogId::new(LeaderId::new(3, 1), 5)),
                    m1_2()
                )),
            },
            Command::UpdateReplicationStreams {
                remove: vec![],
                add: vec![(2, None)]
            },
            // second commit upto the end.
            Command::ReplicateCommitted {
                committed: Some(log_id(3, 6))
            },
            Command::LeaderCommit {
                since: Some(LogId::new(LeaderId::new(3, 1), 4)),
                upto: LogId::new(LeaderId::new(3, 1), 6)
            },
            Command::ReplicateInputEntries { range: 0..3 },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.commands
    );

    Ok(())
}

// TODO(xp): check progress

/// With membership log, fast-commit all if the membership log changes to one voter.
#[test]
fn test_leader_append_entries_fast_commit_if_membership_voter_change_to_1() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m13()));
    eng.state.new_leader();

    // log id will be assigned by eng.
    eng.leader_append_entries(&mut [
        blank(1, 1), //
        Entry {
            log_id: log_id(1, 1),
            payload: EntryPayload::Membership(m1_2()),
        },
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            LogId::new(LeaderId::new(3, 1), 4),
            LogId::new(LeaderId::new(3, 1), 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 1), 6)), eng.state.last_log_id());
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(3, 1), 5)),
                m1_2()
            )),
            effective: Arc::new(EffectiveMembership::new(
                Some(LogId::new(LeaderId::new(3, 1), 5)),
                m1_2()
            ))
        },
        eng.state.membership_state
    );
    assert_eq!(Some(LogId::new(LeaderId::new(3, 1), 6)), eng.state.committed);

    assert_eq!(
        MetricsChangeFlags {
            leader: true,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(
                    Some(LogId::new(LeaderId::new(3, 1), 5)),
                    m1_2()
                )),
            },
            Command::UpdateReplicationStreams {
                remove: vec![(3, None)],
                add: vec![(2, None)]
            },
            // It is correct to commit if the membership change ot a one node cluster.
            Command::ReplicateCommitted {
                committed: Some(log_id(3, 6))
            },
            Command::LeaderCommit {
                since: None,
                upto: LogId::new(LeaderId::new(3, 1), 6)
            },
            Command::ReplicateInputEntries { range: 0..3 },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.commands
    );

    Ok(())
}
