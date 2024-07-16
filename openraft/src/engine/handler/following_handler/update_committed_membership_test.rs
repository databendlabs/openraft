use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::testing::UTConfig;
use crate::engine::Engine;
use crate::testing::log_id;
use crate::type_config::TypeConfigExt;
use crate::utime::UTime;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::Vote;

fn m01() -> Membership<UTConfig> {
    Membership::<UTConfig>::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<UTConfig> {
    Membership::<UTConfig>::new(vec![btreeset! {2,3}], None)
}

fn m34() -> Membership<UTConfig> {
    Membership::<UTConfig>::new(vec![btreeset! {3,4}], None)
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.config.id = 2;
    eng.state.vote = UTime::new(UTConfig::<()>::now(), Vote::new_committed(2, 1));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );

    eng.state.server_state = eng.calc_server_state();
    eng
}

#[test]
fn test_update_committed_membership_at_index_4() -> anyhow::Result<()> {
    // replace effective membership
    let mut eng = eng();

    eng.following_handler()
        .update_committed_membership(EffectiveMembership::new(Some(log_id(3, 1, 4)), m34()));

    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 4)), m34())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 4)), m34()))
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(true, eng.output.take_commands().is_empty());

    Ok(())
}
