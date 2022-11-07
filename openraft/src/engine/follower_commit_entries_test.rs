use std::sync::Arc;

use maplit::btreeset;

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

crate::declare_raft_types!(
    pub(crate) Foo: D=(), R=(), NodeId=u64, Node = (), SD=()
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

fn m01() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {2,3}], None)
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::default();
    eng.state.committed = Some(log_id(1, 1));
    eng.state.membership_state.committed = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()));
    eng
}

#[test]
fn test_follower_commit_entries_empty() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.follower_commit_entries(None, None, &Vec::<Entry<Foo>>::new());

    assert_eq!(Some(log_id(1, 1)), eng.state.committed);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.metrics_flags
    );

    assert_eq!(0, eng.commands.len());

    Ok(())
}

#[test]
fn test_follower_commit_entries_no_update() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.follower_commit_entries(Some(log_id(1, 1)), None, &[blank(2, 4)]);

    assert_eq!(Some(log_id(1, 1)), eng.state.committed);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.metrics_flags
    );

    assert_eq!(0, eng.commands.len());

    Ok(())
}

#[test]
fn test_follower_commit_entries_lt_last_entry() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.follower_commit_entries(Some(log_id(2, 3)), None, &[blank(2, 3)]);

    assert_eq!(Some(log_id(2, 3)), eng.state.committed);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![Command::FollowerCommit {
            already_committed: Some(log_id(1, 1)),
            upto: log_id(2, 3)
        }],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_follower_commit_entries_gt_last_entry() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.follower_commit_entries(Some(log_id(3, 1)), None, &[blank(2, 3)]);

    assert_eq!(Some(log_id(2, 3)), eng.state.committed);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![Command::FollowerCommit {
            already_committed: Some(log_id(1, 1)),
            upto: log_id(2, 3)
        }],
        eng.commands
    );

    Ok(())
}
