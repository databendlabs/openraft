use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    // By default expire the leader lease so that a pre-vote can be granted in these tests.
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(0), Vote::new(2, 1));
    eng.state.server_state = ServerState::Follower;
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 1)]);
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(1, 1, 1)),
        m01(),
    )));
    eng.output.take_commands();

    eng
}

#[test]
fn test_handle_pre_vote_req_rejected_by_leader_lease() -> anyhow::Result<()> {
    let mut eng = eng();
    // An established Leader whose lease has not expired.
    eng.state.vote.update(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 1),
    );
    let vote_before = *eng.state.vote_ref();

    let resp = eng.handle_pre_vote_req(VoteRequest {
        vote: Vote::new(3, 2),
        last_log_id: Some(log_id(2, 1, 3)),
        leadership_transfer: false,
    });

    assert_eq!(
        VoteResponse::new(Vote::new_committed(2, 1), Some(log_id(1, 1, 1)), false),
        resp
    );
    // A pre-vote must not mutate any state.
    assert_eq!(vote_before, *eng.state.vote_ref());
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_handle_pre_vote_req_reject_smaller_last_log_id() -> anyhow::Result<()> {
    let mut eng = eng();
    let vote_before = *eng.state.vote_ref();

    // Local last_log_id is (1,1,1); the candidate has none → reject.
    let resp = eng.handle_pre_vote_req(VoteRequest {
        vote: Vote::new(3, 0),
        last_log_id: None,
        leadership_transfer: false,
    });

    assert_eq!(VoteResponse::new(Vote::new(2, 1), Some(log_id(1, 1, 1)), false), resp);
    assert_eq!(vote_before, *eng.state.vote_ref());
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_handle_pre_vote_req_granted_without_mutation() -> anyhow::Result<()> {
    let mut eng = eng();
    let vote_before = *eng.state.vote_ref();

    // Lease expired (default), the candidate's vote >= local vote and its log is up to date → grant.
    let resp = eng.handle_pre_vote_req(VoteRequest {
        vote: Vote::new(3, 0),
        last_log_id: Some(log_id(1, 1, 1)),
        leadership_transfer: false,
    });

    assert_eq!(VoteResponse::new(Vote::new(2, 1), Some(log_id(1, 1, 1)), true), resp);
    // The defining invariant: granting a pre-vote changes nothing locally.
    assert_eq!(vote_before, *eng.state.vote_ref());
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}
