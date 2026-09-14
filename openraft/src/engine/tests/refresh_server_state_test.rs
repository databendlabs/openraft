use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn m12() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1,2}], [])
}

/// Voters 2,3; node 1 is demoted to a learner.
fn m23_l1() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {2,3}], btreeset! {1})
}

/// Voters 2,3; node 1 is removed.
fn m23() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {2,3}], [])
}

/// Build a Leader of node 1, append `effective` at log-id(2,1,3), then advance the commit point.
fn eng_leader(effective: Membership<u64, ()>, committed: LogIdOf<UTConfig>) -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 1),
    );
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));

    // Establish the Leader under joint before appending either final membership.
    let joint = Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(1, 1, 1)),
        Membership::new_with_defaults(vec![btreeset! {1,2}, btreeset! {2,3}], []),
    ));
    eng.state.membership_state = MembershipState::new(joint.clone(), joint);
    eng.testing_new_leader();
    eng.state.server_state = eng.calc_server_state();

    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(2, 1, 3)),
        effective,
    )));
    eng.state.update_local_committed(&Some(committed));
    eng.output.clear_commands();

    eng
}

#[test]
fn test_refresh_server_state_voter_leader() -> anyhow::Result<()> {
    // The Leader is a voter in the committed effective membership config: nothing changes.
    let mut eng = eng_leader(m12(), log_id(2, 1, 3));

    eng.refresh_server_state();

    assert!(eng.leader.is_some());
    assert_eq!(ServerState::Leader, eng.state.server_state);
    assert!(eng.output.take_commands().is_empty());

    Ok(())
}

#[test]
fn test_refresh_server_state_learner_leader() -> anyhow::Result<()> {
    let mut eng = eng_leader(m23_l1(), log_id(2, 1, 3));

    tracing::info!("--- a retained learner keeps leading after removal from voters commits");
    {
        eng.refresh_server_state();

        assert!(eng.leader.is_some());
        assert_eq!(ServerState::Leader, eng.state.server_state);
        assert!(eng.output.take_commands().is_empty());
    }

    Ok(())
}

#[test]
fn test_refresh_server_state_removed_leader() -> anyhow::Result<()> {
    let mut eng = eng_leader(m23(), log_id(2, 1, 2));

    tracing::info!("--- refresh preserves the committed voter while removal at index 3 is uncommitted");
    {
        eng.refresh_server_state();

        assert!(eng.leader.is_some());
        assert_eq!(ServerState::Leader, eng.state.server_state);
        assert_eq!(&Some(log_id(1, 1, 1)), eng.state.membership_state.committed().log_id());
        assert!(eng.output.take_commands().is_empty());
    }

    tracing::info!("--- committing removal updates membership while the Leader waits for refresh");
    {
        eng.state.update_local_committed(&Some(log_id(2, 1, 3)));

        assert_eq!(
            eng.state.membership_state.effective(),
            eng.state.membership_state.committed()
        );
        assert!(eng.leader.is_some());
        assert_eq!(ServerState::Leader, eng.state.server_state);
    }

    tracing::info!("--- refresh clears leadership after removal is committed");
    {
        eng.refresh_server_state();

        assert!(eng.leader.is_none());
        assert_eq!(ServerState::Learner, eng.state.server_state);
        assert_eq!(
            vec![Command::FailPendingReads, Command::CloseReplicationStreams,],
            eng.output.take_commands()
        );
    }

    Ok(())
}
