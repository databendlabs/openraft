use std::sync::Arc;

use maplit::btreeset;
use pretty_assertions::assert_eq;
use tokio::sync::oneshot;

use crate::core::ServerState;
use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::Respond;
use crate::error::Infallible;
use crate::raft::VoteResponse;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::EffectiveMembership;
use crate::Membership;
use crate::TokioInstant;
use crate::Vote;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

/// Make a sample VoteResponse
fn mk_res() -> Result<VoteResponse<u64>, Infallible> {
    Ok::<VoteResponse<u64>, Infallible>(VoteResponse {
        vote: Vote::new(2, 1),
        vote_granted: false,
        last_log_id: None,
    })
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
fn test_accept_vote_reject_smaller_vote() -> anyhow::Result<()> {
    // When a vote is reject, it generate SendResultCommand and return an error.
    let mut eng = eng();

    let (tx, _rx) = oneshot::channel();
    let resp = eng.vote_handler().accept_vote(&Vote::new(1, 2), tx, |_state, _err| mk_res());

    assert!(resp.is_none());

    let (tx, _rx) = oneshot::channel();
    assert_eq!(
        vec![
            //
            Command::Respond {
                when: None,
                resp: Respond::new(mk_res(), tx)
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

    let (tx, _rx) = oneshot::channel();
    let resp = eng.vote_handler().accept_vote(&Vote::new(3, 3), tx, |_state, _err| mk_res());

    assert!(resp.is_some());

    assert_eq!(
        vec![Command::SaveVote { vote: Vote::new(3, 3) },],
        eng.output.take_commands()
    );

    Ok(())
}
