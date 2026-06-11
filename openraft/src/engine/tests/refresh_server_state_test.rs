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

/// Build a Leader engine of node 1: the committed membership config is `m12()` at log-id(1,1,1),
/// the effective membership config is `effective` at log-id(2,1,3),
/// and logs up to `committed` are committed.
fn eng_leader(effective: Membership<u64, ()>, committed: LogIdOf<UTConfig>) -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.apply_progress_mut().accept(committed);
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 1),
    );
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));

    // Establish the Leader under a membership config that contains node-1, then install the
    // target membership state, mirroring the production order: a Leader exists first, then a
    // membership log entry is appended.
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m12())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m12())),
    );
    eng.testing_new_leader();

    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m12())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 1, 3)), effective)),
    );
    eng.state.server_state = eng.calc_server_state();
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
    // A Leader demoted to a learner but still in the membership config keeps leading.
    let mut eng = eng_leader(m23_l1(), log_id(2, 1, 3));

    eng.refresh_server_state();

    assert!(eng.leader.is_some());
    assert_eq!(ServerState::Leader, eng.state.server_state);
    assert!(eng.output.take_commands().is_empty());

    Ok(())
}

#[test]
fn test_refresh_server_state_removed_leader_uncommitted_membership() -> anyhow::Result<()> {
    // The membership config removing the Leader is not yet committed:
    // the Leader must keep leading to replicate this membership log entry.
    let mut eng = eng_leader(m23(), log_id(2, 1, 2));

    eng.refresh_server_state();

    assert!(eng.leader.is_some());
    assert!(eng.output.take_commands().is_empty());

    Ok(())
}

#[test]
fn test_refresh_server_state_removed_leader_committed_membership() -> anyhow::Result<()> {
    // The membership config removing the Leader is committed: the Leader reverts to a learner.
    let mut eng = eng_leader(m23(), log_id(2, 1, 3));

    eng.refresh_server_state();

    assert!(eng.leader.is_none());
    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(vec![Command::CloseReplicationStreams], eng.output.take_commands());

    Ok(())
}
