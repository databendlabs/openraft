use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft_state::IOId;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

fn m01() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m23() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {2,3}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 1),
    );
    eng.state.apply_progress_mut().accept(log_id(1, 1, 1));
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
    let committed_vote = eng.state.vote_ref().into_committed();
    eng.state.log_progress_mut().accept(IOId::new_log_io(committed_vote, Some(log_id(1, 1, 2))));

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
        vec![
            Command::SaveCommitted {
                committed: log_id(1, 1, 2)
            },
            Command::Apply {
                already_committed: Some(log_id(1, 1, 1)),
                upto: log_id(1, 1, 2),
            }
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_following_handler_commit_entries_le_accepted() -> anyhow::Result<()> {
    let mut eng = eng();
    let committed_vote = eng.state.vote_ref().into_committed();
    eng.state.log_progress_mut().accept(IOId::new_log_io(committed_vote, Some(log_id(3, 1, 4))));

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
            Command::SaveCommitted {
                committed: log_id(2, 1, 3)
            },
            Command::Apply {
                already_committed: Some(log_id(1, 1, 1)),
                upto: log_id(2, 1, 3)
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
