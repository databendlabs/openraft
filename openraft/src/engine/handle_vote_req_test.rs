use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
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

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::<u64, ()>::default();
    eng.state.vote = Vote::new(2, 1);
    eng.state.server_state = ServerState::Candidate;
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
    eng.state.new_leader();
    eng
}

#[test]
fn test_handle_vote_req_reject_smaller_vote() -> anyhow::Result<()> {
    let mut eng = eng();

    let resp = eng.handle_vote_req(VoteRequest {
        vote: Vote::new(1, 2),
        last_log_id: None,
    });

    assert_eq!(
        VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: false,
            last_log_id: None
        },
        resp
    );

    assert_eq!(Vote::new(2, 1), eng.state.vote);
    assert!(eng.state.internal_server_state.is_leading());

    assert_eq!(ServerState::Candidate, eng.state.server_state);
    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.metrics_flags
    );

    assert_eq!(0, eng.commands.len());

    Ok(())
}

#[test]
fn test_handle_vote_req_reject_smaller_last_log_id() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 3)]);

    let resp = eng.handle_vote_req(VoteRequest {
        vote: Vote::new(3, 2),
        last_log_id: Some(log_id(1, 3)),
    });

    assert_eq!(
        VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: false,
            last_log_id: Some(log_id(2, 3))
        },
        resp
    );

    assert_eq!(Vote::new(2, 1), eng.state.vote);
    assert!(eng.state.internal_server_state.is_leading());

    assert_eq!(ServerState::Candidate, eng.state.server_state);
    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.metrics_flags
    );

    assert_eq!(0, eng.commands.len());
    Ok(())
}

#[test]
fn test_handle_vote_req_granted_equal_vote_and_last_log_id() -> anyhow::Result<()> {
    // Equal vote should not emit a SaveVote command.

    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 3)]);

    let resp = eng.handle_vote_req(VoteRequest {
        vote: Vote::new(2, 1),
        last_log_id: Some(log_id(2, 3)),
    });

    assert_eq!(
        VoteResponse {
            vote: Vote::new(2, 1),
            vote_granted: true,
            last_log_id: Some(log_id(2, 3))
        },
        resp
    );

    assert_eq!(Vote::new(2, 1), eng.state.vote);
    assert!(eng.state.internal_server_state.is_following());

    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: true,
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
fn test_handle_vote_req_granted_greater_vote() -> anyhow::Result<()> {
    // A greater vote should emit a SaveVote command.

    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 3)]);

    let resp = eng.handle_vote_req(VoteRequest {
        vote: Vote::new(3, 1),
        last_log_id: Some(log_id(2, 3)),
    });

    assert_eq!(
        VoteResponse {
            // respond the updated vote.
            vote: Vote::new(3, 1),
            vote_granted: true,
            last_log_id: Some(log_id(2, 3))
        },
        resp
    );

    assert_eq!(Vote::new(3, 1), eng.state.vote);
    assert!(eng.state.internal_server_state.is_following());

    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
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
fn test_handle_vote_req_granted_follower_learner_does_not_emit_update_server_state_cmd() -> anyhow::Result<()> {
    // A greater vote should emit a SaveVote command.

    // Learner
    {
        let st = ServerState::Learner;

        let mut eng = eng();
        eng.id = 100; // make it a non-voter
        eng.enter_following();
        eng.state.server_state = st;
        eng.commands = vec![];

        eng.handle_vote_req(VoteRequest {
            vote: Vote::new(3, 1),
            last_log_id: Some(log_id(2, 3)),
        });

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

        eng.handle_vote_req(VoteRequest {
            vote: Vote::new(3, 1),
            last_log_id: Some(log_id(2, 3)),
        });

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
