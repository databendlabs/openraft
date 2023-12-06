use std::sync::Arc;

use maplit::btreeset;

use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::raft_state::Accepted;
use crate::raft_state::LogStateReader;
use crate::testing::log_id;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;

fn m01() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {2,3}], None)
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.state.committed = Some(log_id(1, 1, 1));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng
}

#[test]
fn test_following_handler_commit_entries_empty() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().commit_entries(None);

    assert_eq!(Some(&log_id(1, 1, 1)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_following_handler_commit_entries_ge_accepted() -> anyhow::Result<()> {
    let mut eng = eng();
    let l = eng.state.vote_ref().leader_id();
    eng.state.accepted = Accepted::new(*l, Some(log_id(1, 1, 2)));

    eng.following_handler().commit_entries(Some(log_id(2, 1, 3)));

    assert_eq!(Some(&log_id(1, 1, 2)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(
        vec![Command::Commit {
            seq: 1,
            already_committed: Some(log_id(1, 1, 1)),
            upto: log_id(1, 1, 2),
        }],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_following_handler_commit_entries_le_accepted() -> anyhow::Result<()> {
    let mut eng = eng();
    let l = eng.state.vote_ref().leader_id();
    eng.state.accepted = Accepted::new(*l, Some(log_id(3, 1, 4)));

    eng.following_handler().commit_entries(Some(log_id(2, 1, 3)));

    assert_eq!(Some(&log_id(2, 1, 3)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23()))
        ),
        eng.state.membership_state
    );
    assert_eq!(
        vec![
            //
            Command::Commit {
                seq: 1,
                already_committed: Some(log_id(1, 1, 1)),
                upto: log_id(2, 1, 3)
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
