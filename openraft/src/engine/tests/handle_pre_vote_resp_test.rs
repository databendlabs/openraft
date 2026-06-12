use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn m12() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state
    eng.config.id = 1;
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 1)]);
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(0), Vote::new(0, 0));
    eng.state.server_state = ServerState::Follower;
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(1, 1, 1)),
        m12(),
    )));
    eng
}

#[test]
fn test_handle_pre_vote_resp_quorum_starts_real_election() -> anyhow::Result<()> {
    let mut eng = eng();

    // A pre-vote round in flight at the hypothetical next term, self-granted.
    eng.new_pre_candidate(Vote::new(1, 1));
    eng.pre_candidate_mut().unwrap().grant_by(&1);
    eng.output.take_commands();

    // Peer 2 would grant → quorum {1,2} reached → start a real election.
    eng.handle_pre_vote_resp(2, VoteResponse::new(Vote::new(0, 0), Some(log_id(1, 1, 1)), true));

    assert!(
        eng.pre_candidate_ref().is_none(),
        "pre-vote consumed by the real election"
    );
    assert_eq!(
        Vote::new(1, 1),
        *eng.state.vote_ref(),
        "the real election bumped the vote"
    );
    assert!(eng.candidate_ref().is_some());
    assert_eq!(ServerState::Candidate, eng.state.server_state);

    assert_eq!(
        vec![Command::SaveVote { vote: Vote::new(1, 1) }, Command::SendVote {
            vote_req: VoteRequest::new(Vote::new(1, 1), Some(log_id(1, 1, 1))),
        },],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_handle_pre_vote_resp_reject_keeps_waiting() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.new_pre_candidate(Vote::new(1, 1));
    eng.pre_candidate_mut().unwrap().grant_by(&1);
    eng.output.take_commands();

    let vote_before = *eng.state.vote_ref();

    // Peer 2 rejects → no quorum, no real election, and no state mutation.
    eng.handle_pre_vote_resp(2, VoteResponse::new(Vote::new(0, 0), Some(log_id(1, 1, 1)), false));

    assert!(eng.pre_candidate_ref().is_some(), "still pre-voting");
    assert!(eng.candidate_ref().is_none(), "no real election started");
    assert_eq!(vote_before, *eng.state.vote_ref());
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}
