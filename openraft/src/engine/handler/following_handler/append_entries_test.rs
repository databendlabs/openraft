use std::sync::Arc;

use maplit::btreeset;

use crate::engine::testing::blank_ent;
use crate::engine::testing::UTCfg;
use crate::engine::Engine;
use crate::raft_state::LogStateReader;
use crate::testing::log_id;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::RaftTypeConfig;
use crate::Vote;

fn m01() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {2,3}], None)
}

fn eng() -> Engine<u64, (), <UTCfg as RaftTypeConfig>::Entry> {
    let mut eng = Engine::default();
    eng.state.enable_validate = false; // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.vote.update(*eng.timer.now(), Vote::new_committed(2, 1));
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
fn test_follower_append_entries_update_accepted() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().append_entries(
        Some(log_id(2, 3)),
        vec![
            //
            blank_ent(3, 4),
            blank_ent(3, 5),
        ],
        None,
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
    assert_eq!(Some(&log_id(3, 5)), eng.state.accepted());

    // Update again, accept should not decrease.

    eng.following_handler().append_entries(
        Some(log_id(2, 3)),
        vec![
            //
            blank_ent(3, 4),
        ],
        None,
    );

    assert_eq!(Some(&log_id(3, 5)), eng.state.last_log_id());
    assert_eq!(Some(&log_id(3, 5)), eng.state.accepted());

    Ok(())
}
