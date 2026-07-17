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
fn test_winning_election_cancels_overlapping_pre_vote() -> anyhow::Result<()> {
    let mut eng = eng();

    // Pre-vote A succeeds and starts election A.
    eng.pre_elect();
    eng.handle_pre_vote_resp(2, VoteResponse::new(Vote::new(0, 0), Some(log_id(1, 1, 1)), true));
    eng.candidate_mut().unwrap().grant_by(&1);

    // Election A is still pending when the next election tick starts pre-vote B.
    eng.pre_elect();

    assert_eq!(Vote::new(1, 1), *eng.candidate_ref().unwrap().vote_ref());
    assert_eq!(Vote::new(2, 1), *eng.pre_candidate_ref().unwrap().vote_ref());
    assert_eq!(
        vec![
            Command::SendPreVote {
                vote_req: VoteRequest::new(Vote::new(1, 1), Some(log_id(1, 1, 1))),
            },
            Command::SaveVote { vote: Vote::new(1, 1) },
            Command::SendVote {
                vote_req: VoteRequest::new(Vote::new(1, 1), Some(log_id(1, 1, 1))),
            },
            Command::SendPreVote {
                vote_req: VoteRequest::new(Vote::new(2, 1), Some(log_id(1, 1, 1))),
            },
        ],
        eng.output.take_commands()
    );

    // Election A wins before the delayed response for pre-vote B is received.
    eng.handle_vote_resp(2, VoteResponse::new(Vote::new(1, 1), Some(log_id(1, 1, 1)), true));

    assert_eq!(Vote::new_committed(1, 1), *eng.state.vote_ref());
    assert!(eng.leader.is_some(), "election A established a leader");
    assert!(eng.candidate_ref().is_none(), "election A is complete");
    assert!(
        eng.pre_candidate_ref().is_none(),
        "winning election A must cancel overlapping pre-vote B"
    );

    eng.output.take_commands();

    // A delayed success for B must not start a new election on the leader.
    eng.handle_pre_vote_resp(2, VoteResponse::new(Vote::new(1, 1), Some(log_id(1, 1, 1)), true));

    assert_eq!(Vote::new_committed(1, 1), *eng.state.vote_ref());
    assert_eq!(ServerState::Leader, eng.state.server_state);
    assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());

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

#[test]
fn test_handle_pre_vote_resp_reject_adopts_higher_vote() -> anyhow::Result<()> {
    let mut eng = eng();

    // Local vote at a lower term: term=5, node=3.
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(0), Vote::new(5, 3));

    // Pre-vote round in flight at hypothetical term 6, self-granted by node 1.
    eng.new_pre_candidate(Vote::new(6, 1));
    eng.pre_candidate_mut().unwrap().grant_by(&1);
    eng.output.take_commands();

    // Peer 2 rejects with a strictly higher vote: term=10, node=2.
    eng.handle_pre_vote_resp(2, VoteResponse::new(Vote::new(10, 2), Some(log_id(1, 1, 1)), false));

    // Local vote adopted the responder's higher term (non-committed).
    assert_eq!(&Vote::new(10, 2), eng.state.vote_ref());

    assert_eq!(
        vec![
            Command::SaveVote { vote: Vote::new(10, 2) },
            Command::CloseReplicationStreams,
        ],
        eng.output.take_commands()
    );

    // Adopting the higher term transitions to following state, clearing pre-candidate.
    assert!(
        eng.pre_candidate_ref().is_none(),
        "pre-candidate cleared by term adoption"
    );

    Ok(())
}

#[test]
fn test_handle_pre_vote_resp_reject_equal_vote_unchanged() -> anyhow::Result<()> {
    let mut eng = eng();

    // Local vote: term=5, node=1.
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(0), Vote::new(5, 1));

    eng.new_pre_candidate(Vote::new(6, 1));
    eng.pre_candidate_mut().unwrap().grant_by(&1);
    eng.output.take_commands();

    // Peer 2 rejects with an equal vote — must not disturb the ongoing pre-vote round.
    eng.handle_pre_vote_resp(2, VoteResponse::new(Vote::new(5, 1), Some(log_id(1, 1, 1)), false));

    assert!(eng.pre_candidate_ref().is_some(), "pre-vote round still in flight");
    assert_eq!(&Vote::new(5, 1), eng.state.vote_ref(), "vote unchanged");
    assert_eq!(0, eng.output.take_commands().len(), "no side effects");

    Ok(())
}
