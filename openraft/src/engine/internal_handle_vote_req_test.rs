use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::error::RejectVoteRequest;
use crate::leader::Leader;
use crate::EffectiveMembership;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MetricsChangeFlags;
use crate::Vote;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn m01() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {0,1}], None)
}

fn eng() -> Engine<u64> {
    let mut eng = Engine::<u64>::default();
    eng.state.vote = Vote::new(2, 1);
    eng.state.server_state = ServerState::Candidate;
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
    eng.state.leader = Some(Leader {
        vote_granted_by: Default::default(),
    });
    eng
}

#[test]
fn test_internal_handle_vote_req_reject_smaller_vote() -> anyhow::Result<()> {
    let mut eng = eng();

    let resp = eng.internal_handle_vote_req(&Vote::new(1, 2), &None);

    assert_eq!(Err(RejectVoteRequest::ByVote(Vote::new(2, 1))), resp);

    assert_eq!(Vote::new(2, 1), eng.state.vote);
    assert!(eng.state.leader.is_some());

    assert_eq!(ServerState::Candidate, eng.state.server_state);
    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: false
        },
        eng.metrics_flags
    );

    assert_eq!(0, eng.commands.len());

    Ok(())
}

#[test]
fn test_internal_handle_vote_req_reject_smaller_last_log_id() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.last_log_id = Some(log_id(2, 3));

    let resp = eng.internal_handle_vote_req(&Vote::new(3, 2), &Some(log_id(1, 3)));

    assert_eq!(Err(RejectVoteRequest::ByLastLogId(Some(log_id(2, 3)))), resp);

    assert_eq!(Vote::new(2, 1), eng.state.vote);
    assert!(eng.state.leader.is_some());

    assert_eq!(ServerState::Candidate, eng.state.server_state);
    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: false
        },
        eng.metrics_flags
    );

    assert_eq!(0, eng.commands.len());
    Ok(())
}

#[test]
fn test_internal_handle_vote_req_committed_vote() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.last_log_id = Some(log_id(2, 3));

    let resp = eng.internal_handle_vote_req(&Vote::new_committed(3, 2), &None);

    assert_eq!(Ok(()), resp);

    assert_eq!(Vote::new_committed(3, 2), eng.state.vote);
    assert!(eng.state.leader.is_none());

    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            //
            Command::InstallElectionTimer {},
            Command::RejectElection {},
            Command::SaveVote {
                vote: Vote::new_committed(3, 2)
            },
            Command::UpdateServerState {
                server_state: ServerState::Follower
            }
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_internal_handle_vote_req_granted_equal_vote_and_last_log_id() -> anyhow::Result<()> {
    // Equal vote should not emit a SaveVote command.

    let mut eng = eng();
    eng.state.last_log_id = Some(log_id(2, 3));

    let resp = eng.internal_handle_vote_req(&Vote::new(2, 1), &Some(log_id(2, 3)));

    assert_eq!(Ok(()), resp);

    assert_eq!(Vote::new(2, 1), eng.state.vote);
    assert!(eng.state.leader.is_none());

    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            //
            Command::InstallElectionTimer {},
            Command::UpdateServerState {
                server_state: ServerState::Follower
            }
        ],
        eng.commands
    );
    Ok(())
}

#[test]
fn test_internal_handle_vote_req_granted_greater_vote() -> anyhow::Result<()> {
    // A greater vote should emit a SaveVote command.

    let mut eng = eng();
    eng.state.last_log_id = Some(log_id(2, 3));

    let resp = eng.internal_handle_vote_req(&Vote::new(3, 1), &Some(log_id(2, 3)));

    assert_eq!(Ok(()), resp);

    assert_eq!(Vote::new(3, 1), eng.state.vote);
    assert!(eng.state.leader.is_none());

    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::InstallElectionTimer {},
            Command::SaveVote { vote: Vote::new(3, 1) },
            Command::UpdateServerState {
                server_state: ServerState::Follower
            }
        ],
        eng.commands
    );
    Ok(())
}

#[test]
fn test_internal_handle_vote_req_granted_follower_learner_does_not_emit_update_server_state_cmd() -> anyhow::Result<()>
{
    // A greater vote should emit a SaveVote command.

    for st in [ServerState::Learner, ServerState::Follower] {
        let mut eng = eng();
        eng.state.server_state = st;

        let resp = eng.internal_handle_vote_req(&Vote::new(3, 1), &Some(log_id(2, 3)));

        assert_eq!(Ok(()), resp);

        assert_eq!(st, eng.state.server_state);
        assert_eq!(
            vec![
                //
                Command::InstallElectionTimer {},
                Command::SaveVote { vote: Vote::new(3, 1) },
            ],
            eng.commands
        );
    }
    Ok(())
}
