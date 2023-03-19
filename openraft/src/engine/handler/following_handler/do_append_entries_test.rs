use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
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

fn m34() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {3,4}], None)
}

fn m45() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {4,5}], None)
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::default();
    eng.state.enable_validate = false; // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.log_ids.append(log_id(1, 1));
    eng.state.log_ids.append(log_id(2, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
    );
    eng.state.server_state = eng.calc_server_state();
    eng
}

#[test]
fn test_follower_do_append_entries_empty() -> anyhow::Result<()> {
    let mut eng = eng();

    // Neither of these two will update anything.
    eng.following_handler().do_append_entries(&Vec::<Entry<Foo>>::new(), 0);
    eng.following_handler().do_append_entries(&[blank(3, 4)], 1);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(2, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);

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
fn test_follower_do_append_entries_no_membership_entries() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().do_append_entries(
        &[
            blank(100, 100), // just be ignored
            blank(3, 4),
        ],
        1,
    );

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            log_id(3, 4),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(3, 4)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![Command::AppendInputEntries { range: 1..2 }, //
            ],
        eng.output.commands
    );

    Ok(())
}

#[test]
fn test_follower_do_append_entries_one_membership_entry() -> anyhow::Result<()> {
    // - The membership entry in the input becomes effective membership. The previous effective becomes
    //   committed.
    // - Follower become Learner, since it is not in the new effective membership.
    let mut eng = eng();
    eng.config.id = 2; // make it a member, the become learner

    eng.following_handler().do_append_entries(
        &[
            blank(3, 3), // ignored
            blank(3, 3), // ignored
            blank(3, 3), // ignored
            blank(3, 4),
            Entry {
                log_id: log_id(3, 5),
                payload: EntryPayload::<Foo>::Membership(m34()),
            },
        ],
        3,
    );

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
            log_id(3, 4),
            log_id(3, 5),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(3, 5)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m34())),
        ),
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
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 3..5 }, //
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m34())),
            },
        ],
        eng.output.commands
    );

    Ok(())
}

#[test]
fn test_follower_do_append_entries_three_membership_entries() -> anyhow::Result<()> {
    // - The last 2 of the 3 membership entries take effect.
    // - A learner become follower.

    let mut eng = eng();
    eng.config.id = 5; // make it a learner, then become follower
    eng.state.server_state = eng.calc_server_state();

    eng.following_handler().do_append_entries(
        &[
            Entry {
                log_id: log_id(3, 4),
                payload: EntryPayload::<Foo>::Membership(m01()),
            }, // ignored
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
        ],
        1,
    );

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
    assert_eq!(Some(&log_id(4, 7)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(4, 6)), m34())),
            Arc::new(EffectiveMembership::new(Some(log_id(4, 7)), m45())),
        ),
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
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 1..5 }, //
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(4, 7)), m45())),
            },
        ],
        eng.output.commands
    );

    Ok(())
}
