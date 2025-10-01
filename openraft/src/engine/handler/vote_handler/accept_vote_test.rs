use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::EffectiveMembership;
use crate::Membership;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::Engine;
use crate::engine::Respond;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::VoteResponse;
use crate::raft_state::IOId;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;

fn m01() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {0,1}], [])
}

/// Make a sample VoteResponse
fn mk_res(granted: bool) -> VoteResponse<UTConfig> {
    VoteResponse::new(Vote::new(2, 1), None, granted)
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 0;
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(2, 1));
    eng.state.server_state = ServerState::Candidate;
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())));

    eng
}

#[test]
fn test_accept_vote_reject_smaller_vote() -> anyhow::Result<()> {
    // When a vote is reject, it generate SendResultCommand and return an error.
    let mut eng = eng();
    eng.output.take_commands();

    let (tx, _rx) = UTConfig::<()>::oneshot();
    let resp = eng.vote_handler().accept_vote(&Vote::new(1, 2), tx, |_state, _err| mk_res(false));

    assert!(resp.is_none());

    let (tx, _rx) = UTConfig::<()>::oneshot();
    assert_eq!(
        vec![
            //
            Command::Respond {
                when: Some(Condition::IOFlushed {
                    io_id: IOId::new(&Vote::new(2, 1))
                }),
                resp: Respond::new(mk_res(false), tx)
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_accept_vote_granted_greater_vote() -> anyhow::Result<()> {
    // When a vote is accepted, it generate SaveVote command and return an Ok.
    let mut eng = eng();
    eng.output.take_commands();

    let (tx, _rx) = UTConfig::<()>::oneshot();
    let resp = eng.vote_handler().accept_vote(&Vote::new(3, 3), tx, |_state, _err| mk_res(true));

    assert!(resp.is_some());

    assert_eq!(
        vec![Command::SaveVote { vote: Vote::new(3, 3) },],
        eng.output.take_commands()
    );

    Ok(())
}
