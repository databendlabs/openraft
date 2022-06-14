use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
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

fn m23() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {2,3}], None)
}

fn m34() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {3,4}], None)
}

fn m45() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {4,5}], None)
}

fn eng() -> Engine<u64> {
    let mut eng = Engine::<u64> {
        id: 2, // make it a member
        ..Default::default()
    };
    eng.state.server_state = ServerState::Candidate;
    eng.state.log_ids.append(log_id(1, 1));
    eng.state.log_ids.append(log_id(2, 3));
    eng.state.last_log_id = Some(log_id(2, 3));
    eng.state.membership_state.committed = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()));
    eng
}

#[test]
fn test_follower_append_entries_empty() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.follower_append_entries(&Vec::<Entry<Foo>>::new());

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
    assert_eq!(ServerState::Candidate, eng.state.server_state);

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
fn test_follower_append_entries_no_membership_entries() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.follower_append_entries(&[
        //
        blank(3, 4),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            log_id(3, 4),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(log_id(3, 4)), eng.state.last_log_id);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );
    assert_eq!(ServerState::Candidate, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..1 },
            Command::MoveInputCursorBy { n: 1 }
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_follower_append_entries_one_membership_entry() -> anyhow::Result<()> {
    // - The membership entry in the input becomes effective membership. The previous effective becomes committed.
    // - Follower become Learner, since it is not in the new effective membership.
    let mut eng = eng();
    eng.id = 2; // make it a member, the become learner

    eng.follower_append_entries(&[
        //
        blank(3, 4),
        Entry {
            log_id: log_id(3, 5),
            payload: EntryPayload::<Foo>::Membership(m34()),
        },
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            log_id(3, 4),
            log_id(3, 5),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(log_id(3, 5)), eng.state.last_log_id);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m34()))
        },
        eng.state.membership_state,
        "previous effective become committed"
    );
    assert_eq!(
        ServerState::Learner,
        eng.state.server_state,
        "not in membership, become learner"
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
            Command::AppendInputEntries { range: 0..2 },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m34())),
            },
            Command::UpdateServerState {
                server_state: ServerState::Learner
            },
            Command::MoveInputCursorBy { n: 2 }
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_follower_append_entries_three_membership_entries() -> anyhow::Result<()> {
    // - The last 2 of the 3 membership entries take effect.
    // - A learner become follower.

    let mut eng = eng();
    eng.id = 5; // make it a learner, then become follower

    eng.follower_append_entries(&[
        //
        blank(3, 4),
        Entry {
            log_id: log_id(3, 5),
            payload: EntryPayload::<Foo>::Membership(m01()),
        },
        Entry {
            log_id: log_id(4, 6),
            payload: EntryPayload::<Foo>::Membership(m34()),
        },
        Entry {
            log_id: log_id(4, 7),
            payload: EntryPayload::<Foo>::Membership(m45()),
        },
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            log_id(3, 4),
            log_id(4, 6),
            log_id(4, 7),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(log_id(4, 7)), eng.state.last_log_id);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(4, 6)), m34())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(4, 7)), m45()))
        },
        eng.state.membership_state,
        "seen 3 membership, the last 2 become committed and effective"
    );
    assert_eq!(
        ServerState::Follower,
        eng.state.server_state,
        "in membership, become follower"
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
            Command::AppendInputEntries { range: 0..4 },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(4, 7)), m45())),
            },
            Command::UpdateServerState {
                server_state: ServerState::Follower
            },
            Command::MoveInputCursorBy { n: 4 }
        ],
        eng.commands
    );

    Ok(())
}
