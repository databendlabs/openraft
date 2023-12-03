use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::error::RejectVoteRequest;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::EffectiveMembership;
use crate::Membership;
use crate::TokioInstant;
use crate::Vote;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 0;
    eng.state.vote = UTime::new(TokioInstant::now(), Vote::new(2, 1));
    eng.state.server_state = ServerState::Candidate;
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())));

    eng.vote_handler().become_leading();
    eng
}

#[test]
fn test_handle_message_vote_reject_smaller_vote() -> anyhow::Result<()> {
    let mut eng = eng();

    let resp = eng.vote_handler().update_vote(&Vote::new(1, 2));

    assert_eq!(Err(RejectVoteRequest::ByVote(Vote::new(2, 1))), resp);

    assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
    assert!(eng.internal_server_state.is_leading());

    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_handle_message_vote_committed_vote() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 1, 3)]);
    let now = TokioInstant::now();

    let resp = eng.vote_handler().update_vote(&Vote::new_committed(3, 2));

    assert_eq!(Ok(()), resp);

    assert_eq!(Vote::new_committed(3, 2), *eng.state.vote_ref());
    assert!(eng.internal_server_state.is_following());

    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert!(Some(now) <= eng.state.vote_last_modified());
    assert!(eng.state.vote_last_modified() <= Some(now + Duration::from_millis(20)));
    assert_eq!(
        vec![Command::SaveVote {
            vote: Vote::new_committed(3, 2)
        },],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_handle_message_vote_granted_equal_vote() -> anyhow::Result<()> {
    // Equal vote should not emit a SaveVote command.

    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 1, 3)]);
    let now = TokioInstant::now();

    let resp = eng.vote_handler().update_vote(&Vote::new(2, 1));

    assert_eq!(Ok(()), resp);

    assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
    assert!(eng.internal_server_state.is_following());

    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert!(Some(now) <= eng.state.vote_last_modified());
    assert!(eng.state.vote_last_modified() <= Some(now + Duration::from_millis(20)));

    assert!(eng.output.take_commands().is_empty());
    Ok(())
}

#[test]
fn test_handle_message_vote_granted_greater_vote() -> anyhow::Result<()> {
    // A greater vote should emit a SaveVote command.

    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 1, 3)]);

    let resp = eng.vote_handler().update_vote(&Vote::new(3, 1));

    assert_eq!(Ok(()), resp);

    assert_eq!(Vote::new(3, 1), *eng.state.vote_ref());
    assert!(eng.internal_server_state.is_following());

    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        vec![Command::SaveVote { vote: Vote::new(3, 1) },],
        eng.output.take_commands()
    );
    Ok(())
}

#[test]
fn test_handle_message_vote_granted_follower_learner_does_not_emit_update_server_state_cmd() -> anyhow::Result<()> {
    // A greater vote should emit a SaveVote command.

    // Learner
    {
        let st = ServerState::Learner;

        let mut eng = eng();
        eng.config.id = 100; // make it a non-voter
        eng.vote_handler().become_following();
        eng.state.server_state = st;
        eng.output.clear_commands();

        let resp = eng.vote_handler().update_vote(&Vote::new(3, 1));

        assert_eq!(Ok(()), resp);

        assert_eq!(st, eng.state.server_state);
        assert_eq!(
            vec![
                //
                Command::SaveVote { vote: Vote::new(3, 1) },
            ],
            eng.output.take_commands()
        );
    }
    // Follower
    {
        let st = ServerState::Follower;

        let mut eng = eng();
        eng.config.id = 0; // make it a voter
        eng.vote_handler().become_following();
        eng.state.server_state = st;
        eng.output.clear_commands();

        let resp = eng.vote_handler().update_vote(&Vote::new(3, 1));

        assert_eq!(Ok(()), resp);

        assert_eq!(st, eng.state.server_state);
        assert_eq!(
            vec![
                //
                Command::SaveVote { vote: Vote::new(3, 1) },
            ],
            eng.output.take_commands()
        );
    }
    Ok(())
}
