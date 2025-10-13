use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use crate::EffectiveMembership;
use crate::Entry;
use crate::EntryPayload;
use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::entry::RaftEntry;
use crate::raft_state::LogStateReader;
use crate::testing::blank_ent;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

fn m01() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m23() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {2,3}], [])
}

fn m34() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {3,4}], [])
}

fn m45() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {4,5}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 1),
    );
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng.state.server_state = eng.calc_server_state();
    eng
}

#[test]
fn test_follower_do_append_entries_no_membership_entries() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.vote = Leased::without_last_update(Vote::new_committed(1, 1));

    eng.following_handler().do_append_entries(vec![blank_ent(3, 1, 4)]);

    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
            log_id(3, 1, 4),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(3, 1, 4)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        vec![
            //
            Command::AppendEntries {
                committed_vote: Vote::new(1, 1).into_committed(),
                entries: vec![blank_ent(3, 1, 4)]
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
    // - Follower becomes Learner, since it is not in the new effective membership.
    let mut eng = eng();
    eng.config.id = 2; // make it a member, the become learner
    eng.state.vote = Leased::without_last_update(Vote::new_committed(1, 1));

    eng.following_handler().do_append_entries(vec![blank_ent(3, 1, 4), Entry::<UTConfig> {
        log_id: log_id(3, 1, 5),
        payload: EntryPayload::<UTConfig>::Membership(m34()),
    }]);

    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
            log_id(3, 1, 4),
            log_id(3, 1, 5),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(3, 1, 5)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 5)), m34())),
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
        vec![Command::AppendEntries {
            committed_vote: Vote::new(1, 1).into_committed(),
            entries: vec![
                //
                blank_ent(3, 1, 4),
                Entry::<UTConfig> {
                    log_id: log_id(3, 1, 5),
                    payload: EntryPayload::<UTConfig>::Membership(m34()),
                },
            ]
        },],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_follower_do_append_entries_three_membership_entries() -> anyhow::Result<()> {
    // - The last 2 of the 3 membership entries take effect.
    // - A learner becomes follower.

    let mut eng = eng();
    eng.config.id = 5; // make it a learner, then become follower
    eng.state.server_state = eng.calc_server_state();
    eng.state.vote = Leased::without_last_update(Vote::new_committed(1, 1));

    eng.following_handler().do_append_entries(vec![
        blank_ent(3, 1, 4),
        Entry::<UTConfig>::new_membership(log_id(3, 1, 5), m01()),
        Entry::<UTConfig>::new_membership(log_id(4, 1, 6), m34()),
        Entry::<UTConfig>::new_membership(log_id(4, 1, 7), m45()),
    ]);

    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
            log_id(3, 1, 4),
            log_id(4, 1, 6),
            log_id(4, 1, 7),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(4, 1, 7)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(4, 1, 6)), m34())),
            Arc::new(EffectiveMembership::new(Some(log_id(4, 1, 7)), m45())),
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
        vec![Command::AppendEntries {
            committed_vote: Vote::new(1, 1).into_committed(),
            entries: vec![
                blank_ent(3, 1, 4),
                Entry::<UTConfig>::new_membership(log_id(3, 1, 5), m01()),
                Entry::<UTConfig>::new_membership(log_id(4, 1, 6), m34()),
                Entry::<UTConfig>::new_membership(log_id(4, 1, 7), m45()),
            ]
        },],
        eng.output.take_commands()
    );

    Ok(())
}
