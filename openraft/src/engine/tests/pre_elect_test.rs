use std::collections::BTreeSet;
use std::sync::Arc;

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
use crate::type_config::alias::StoredMembershipOf;

fn m1() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1}], [])
}

fn m12() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1,2}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.log_ids = LogIdList::new(None, [log_id(0, 0, 0)]);
    eng.state.enable_validation(false); // Disable validation for incomplete state
    eng
}

#[test]
fn test_pre_elect_multi_node_no_mutation() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.config.id = 1;
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(0, 1, 1)),
        m12(),
    )));
    eng.state.log_ids = LogIdList::new(None, vec![log_id(1, 1, 1)]);

    let vote_before = *eng.state.vote_ref();
    let server_state_before = eng.state.server_state;

    eng.pre_elect();

    // A pre-vote must NOT bump the term, persist a vote, or change server state.
    assert_eq!(vote_before, *eng.state.vote_ref());
    assert_eq!(server_state_before, eng.state.server_state);
    assert!(eng.candidate_ref().is_none(), "no real candidate during pre-vote");

    // A pre-candidate at the hypothetical next term, having granted itself.
    assert_eq!(Vote::new(1, 1), *eng.pre_candidate_ref().unwrap().vote_ref());
    assert_eq!(
        btreeset! {1},
        eng.pre_candidate_ref().unwrap().granters().collect::<BTreeSet<_>>()
    );

    // Only SendPreVote is emitted — crucially, NO SaveVote and NO SendVote.
    assert_eq!(
        vec![Command::SendPreVote {
            vote_req: VoteRequest::new(Vote::new(1, 1), Some(log_id(1, 1, 1)))
        }],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_pre_elect_single_node_starts_real_election() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.config.id = 1;
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(0, 1, 1)),
        m1(),
    )));

    eng.pre_elect();

    // A single voter wins its own pre-vote and proceeds straight to a real election.
    assert_eq!(Vote::new(1, 1), *eng.state.vote_ref());
    assert!(eng.candidate_ref().is_some());
    assert!(
        eng.pre_candidate_ref().is_none(),
        "pre-vote consumed by the real election"
    );
    assert_eq!(ServerState::Candidate, eng.state.server_state);

    assert_eq!(
        vec![Command::SaveVote { vote: Vote::new(1, 1) }, Command::SendVote {
            vote_req: VoteRequest::new(Vote::new(1, 1), Some(log_id(0, 0, 0))),
        },],
        eng.output.take_commands()
    );

    Ok(())
}
