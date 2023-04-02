use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::testing::UTCfg;
use crate::engine::Command;
use crate::engine::Engine;
use crate::entry::RaftEntry;
use crate::raft_state::LogStateReader;
use crate::testing::log_id;
use crate::EffectiveMembership;
use crate::Entry;
use crate::EntryPayload;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;
use crate::RaftTypeConfig;

fn blank(term: u64, index: u64) -> Entry<UTCfg> {
    Entry {
        log_id: log_id(term, index),
        payload: EntryPayload::<UTCfg>::Blank,
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

fn eng() -> Engine<u64, (), <UTCfg as RaftTypeConfig>::Entry> {
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

    eng.following_handler().do_append_entries(Vec::<Entry<UTCfg>>::new(), 0);
    eng.following_handler().do_append_entries(vec![blank(3, 4)], 1);

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

    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_follower_do_append_entries_no_membership_entries() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().do_append_entries(
        vec![
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
        vec![
            //
            Command::AppendInputEntries {
                entries: vec![blank(3, 4)]
            },
        ],
        eng.output.take_commands()
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
        vec![
            blank(3, 3), // ignored
            blank(3, 3), // ignored
            blank(3, 3), // ignored
            blank(3, 4),
            Entry::<UTCfg> {
                log_id: log_id(3, 5),
                payload: EntryPayload::<UTCfg>::Membership(m34()),
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
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m34())),
            },
            Command::AppendInputEntries {
                entries: vec![
                    //
                    blank(3, 4),
                    Entry::<UTCfg> {
                        log_id: log_id(3, 5),
                        payload: EntryPayload::<UTCfg>::Membership(m34()),
                    },
                ]
            },
        ],
        eng.output.take_commands()
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
        vec![
            Entry::<UTCfg>::new_membership(log_id(3, 4), m01()), // ignored
            blank(3, 4),
            Entry::<UTCfg>::new_membership(log_id(3, 5), m01()),
            Entry::<UTCfg>::new_membership(log_id(4, 6), m34()),
            Entry::<UTCfg>::new_membership(log_id(4, 7), m45()),
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
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(4, 7)), m45())),
            },
            Command::AppendInputEntries {
                entries: vec![
                    blank(3, 4),
                    Entry::<UTCfg>::new_membership(log_id(3, 5), m01()),
                    Entry::<UTCfg>::new_membership(log_id(4, 6), m34()),
                    Entry::<UTCfg>::new_membership(log_id(4, 7), m45()),
                ]
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
