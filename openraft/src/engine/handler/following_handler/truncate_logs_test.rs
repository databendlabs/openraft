use std::sync::Arc;

use maplit::btreeset;

use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::raft_state::LogStateReader;
use crate::testing::log_id;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::ServerState;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

fn m12() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {2,3}], None)
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.log_ids = LogIdList::new(vec![
        log_id(2, 1, 2), //
        log_id(4, 1, 4),
        log_id(4, 1, 6),
    ]);
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );

    eng.state.server_state = ServerState::Follower;
    eng
}

#[test]
fn test_truncate_logs_since_3() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.server_state = eng.calc_server_state();

    eng.following_handler().truncate_logs(3);

    assert_eq!(Some(&log_id(2, 1, 2)), eng.state.last_log_id());
    assert_eq!(&[log_id(2, 1, 2)], eng.state.log_ids.key_log_ids());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        vec![
            //
            Command::DeleteConflictLog { since: log_id(2, 1, 3) },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_truncate_logs_since_4() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().truncate_logs(4);

    assert_eq!(Some(&log_id(2, 1, 3)), eng.state.last_log_id());
    assert_eq!(&[log_id(2, 1, 2), log_id(2, 1, 3)], eng.state.log_ids.key_log_ids());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(
        vec![Command::DeleteConflictLog { since: log_id(4, 1, 4) }],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_truncate_logs_since_5() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().truncate_logs(5);

    assert_eq!(Some(&log_id(4, 1, 4)), eng.state.last_log_id());
    assert_eq!(&[log_id(2, 1, 2), log_id(4, 1, 4)], eng.state.log_ids.key_log_ids());
    assert_eq!(
        vec![Command::DeleteConflictLog { since: log_id(4, 1, 5) }],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_truncate_logs_since_6() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().truncate_logs(6);

    assert_eq!(Some(&log_id(4, 1, 5)), eng.state.last_log_id());
    assert_eq!(
        &[log_id(2, 1, 2), log_id(4, 1, 4), log_id(4, 1, 5)],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(
        vec![Command::DeleteConflictLog { since: log_id(4, 1, 6) }],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_truncate_logs_since_7() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().truncate_logs(7);

    assert_eq!(Some(&log_id(4, 1, 6)), eng.state.last_log_id());
    assert_eq!(
        &[log_id(2, 1, 2), log_id(4, 1, 4), log_id(4, 1, 6)],
        eng.state.log_ids.key_log_ids()
    );
    assert!(eng.output.take_commands().is_empty());

    Ok(())
}

#[test]
fn test_truncate_logs_since_8() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().truncate_logs(8);

    assert_eq!(Some(&log_id(4, 1, 6)), eng.state.last_log_id());
    assert_eq!(
        &[log_id(2, 1, 2), log_id(4, 1, 4), log_id(4, 1, 6)],
        eng.state.log_ids.key_log_ids()
    );
    assert!(eng.output.take_commands().is_empty());

    Ok(())
}

#[test]
fn test_truncate_logs_revert_effective_membership() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(4, 1, 4)), m12())),
    );
    eng.state.server_state = eng.calc_server_state();

    eng.following_handler().truncate_logs(4);

    assert_eq!(Some(&log_id(2, 1, 3)), eng.state.last_log_id());
    assert_eq!(&[log_id(2, 1, 2), log_id(2, 1, 3)], eng.state.log_ids.key_log_ids());
    assert_eq!(
        vec![
            //
            Command::DeleteConflictLog { since: log_id(4, 1, 4) },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
