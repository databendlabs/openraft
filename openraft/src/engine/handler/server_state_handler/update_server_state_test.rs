use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::ServerState;
use crate::Vote;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;

fn m01() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m123() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 2),
    );
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m123())),
    );
    eng.state.server_state = eng.state.calc_server_state(&eng.config.id);

    eng
}
#[test]
fn test_update_server_state_if_changed() -> anyhow::Result<()> {
    //
    let mut eng = eng();
    let mut ssh = eng.server_state_handler();

    // Leader become follower
    {
        assert_eq!(ServerState::Leader, ssh.state.server_state);

        ssh.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(2, 100));
        ssh.update_server_state_if_changed();

        assert_eq!(ServerState::Follower, ssh.state.server_state);
    }

    // TODO(3): add more test,
    //          after migrating to the no-step-down leader:
    //          A leader keeps working after it is removed from the voters.
    Ok(())
}
