use std::sync::Arc;

use maplit::btreeset;

use crate::engine::testing::UTConfig;
use crate::engine::Engine;
use crate::raft_state::LogStateReader;
use crate::testing::blank_ent;
use crate::testing::log_id;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::TokioInstant;
use crate::Vote;

fn m01() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {2,3}], None)
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.vote.update(TokioInstant::now(), Vote::new_committed(2, 1));
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
fn test_follower_append_entries_update_accepted() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().append_entries(Some(log_id(2, 1, 3)), vec![
        //
        blank_ent(3, 1, 4),
        blank_ent(3, 1, 5),
    ]);

    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
            log_id(3, 1, 4),
            log_id(3, 1, 5),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(3, 1, 5)), eng.state.accepted());

    // Update again, accept should not decrease.

    eng.following_handler().append_entries(Some(log_id(2, 1, 3)), vec![
        //
        blank_ent(3, 1, 4),
    ]);

    assert_eq!(Some(&log_id(3, 1, 5)), eng.state.last_log_id());
    assert_eq!(Some(&log_id(3, 1, 5)), eng.state.accepted());

    Ok(())
}
