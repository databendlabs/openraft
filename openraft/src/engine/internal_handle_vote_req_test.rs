use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::error::RejectVoteRequest;
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
    eng.state.new_leader();
    eng
}

#[test]
fn test_handle_vote_change_reject_smaller_vote() -> anyhow::Result<()> {
    let mut eng = eng();

    let resp = eng.handle_vote_change(&Vote::new(1, 2));

    assert_eq!(Err(RejectVoteRequest::ByVote(Vote::new(2, 1))), resp);

    assert_eq!(Vote::new(2, 1), eng.state.vote);
    assert!(eng.state.internal_server_state.is_leading());

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
fn test_handle_vote_change_committed_vote() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 3)]);

    let resp = eng.handle_vote_change(&Vote::new_committed(3, 2));

    assert_eq!(Ok(()), resp);

    assert_eq!(Vote::new_committed(3, 2), eng.state.vote);
    assert!(eng.state.internal_server_state.is_following());

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
            Command::SaveVote {
                vote: Vote::new_committed(3, 2)
            },
            Command::InstallElectionTimer { can_be_leader: false },
            Command::RejectElection {},
            Command::UpdateServerState {
                server_state: ServerState::Follower
            }
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_handle_vote_change_granted_equal_vote() -> anyhow::Result<()> {
    // Equal vote should not emit a SaveVote command.

    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 3)]);

    let resp = eng.handle_vote_change(&Vote::new(2, 1));

    assert_eq!(Ok(()), resp);

    assert_eq!(Vote::new(2, 1), eng.state.vote);
    assert!(eng.state.internal_server_state.is_following());

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
            Command::InstallElectionTimer { can_be_leader: true },
            Command::UpdateServerState {
                server_state: ServerState::Follower
            }
        ],
        eng.commands
    );
    Ok(())
}

#[test]
fn test_handle_vote_change_granted_greater_vote() -> anyhow::Result<()> {
    // A greater vote should emit a SaveVote command.

    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 3)]);

    let resp = eng.handle_vote_change(&Vote::new(3, 1));

    assert_eq!(Ok(()), resp);

    assert_eq!(Vote::new(3, 1), eng.state.vote);
    assert!(eng.state.internal_server_state.is_following());

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
            Command::SaveVote { vote: Vote::new(3, 1) },
            Command::InstallElectionTimer { can_be_leader: true },
            Command::UpdateServerState {
                server_state: ServerState::Follower
            }
        ],
        eng.commands
    );
    Ok(())
}

#[test]
fn test_handle_vote_change_granted_follower_learner_does_not_emit_update_server_state_cmd() -> anyhow::Result<()> {
    // A greater vote should emit a SaveVote command.

    // Learner
    {
        let st = ServerState::Learner;

        let mut eng = eng();
        eng.id = 100; // make it a non-voter
        eng.enter_following();
        eng.state.server_state = st;
        eng.commands = vec![];

        let resp = eng.handle_vote_change(&Vote::new(3, 1));

        assert_eq!(Ok(()), resp);

        assert_eq!(st, eng.state.server_state);
        assert_eq!(
            vec![
                //
                Command::SaveVote { vote: Vote::new(3, 1) },
                Command::InstallElectionTimer { can_be_leader: true },
            ],
            eng.commands
        );
    }
    // Follower
    {
        let st = ServerState::Follower;

        let mut eng = eng();
        eng.id = 0; // make it a voter
        eng.enter_following();
        eng.state.server_state = st;
        eng.commands = vec![];

        let resp = eng.handle_vote_change(&Vote::new(3, 1));

        assert_eq!(Ok(()), resp);

        assert_eq!(st, eng.state.server_state);
        assert_eq!(
            vec![
                //
                Command::SaveVote { vote: Vote::new(3, 1) },
                Command::InstallElectionTimer { can_be_leader: true },
            ],
            eng.commands
        );
    }
    Ok(())
}
