use std::sync::Arc;

use maplit::btreeset;

use crate::engine::Command;
use crate::engine::Engine;
use crate::raft_state::LogStateReader;
use crate::EffectiveMembership;
use crate::Entry;
use crate::EntryPayload;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;

crate::declare_raft_types!(
    pub(crate) Foo: D=(), R=(), NodeId=u64, Node = (), Entry = crate::Entry<Foo>
);

use crate::testing::log_id;

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
    eng.state.enable_validate = false; // Disable validation for incomplete state

    eng.state.committed = Some(log_id(1, 1));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
    );
    eng
}

#[test]
fn test_follower_commit_entries_empty() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().commit_entries(None, None, &Vec::<Entry<Foo>>::new());

    assert_eq!(Some(&log_id(1, 1)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(0, eng.output.commands.len());

    Ok(())
}

#[test]
fn test_follower_commit_entries_no_update() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().commit_entries(Some(log_id(1, 1)), None, &[blank(2, 4)]);

    assert_eq!(Some(&log_id(1, 1)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(0, eng.output.commands.len());

    Ok(())
}

#[test]
fn test_follower_commit_entries_lt_last_entry() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().commit_entries(Some(log_id(2, 3)), None, &[blank(2, 3)]);

    assert_eq!(Some(&log_id(2, 3)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::FollowerCommit {
                already_committed: Some(log_id(1, 1)),
                upto: log_id(2, 3)
            }, //
        ],
        eng.output.commands
    );

    Ok(())
}

#[test]
fn test_follower_commit_entries_gt_last_entry() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().commit_entries(Some(log_id(3, 1)), None, &[blank(2, 3)]);

    assert_eq!(Some(&log_id(2, 3)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        ),
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::FollowerCommit {
                already_committed: Some(log_id(1, 1)),
                upto: log_id(2, 3)
            }, //
        ],
        eng.output.commands
    );

    Ok(())
}
