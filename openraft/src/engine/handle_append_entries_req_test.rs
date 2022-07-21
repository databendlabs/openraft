use std::sync::Arc;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::raft::AppendEntriesResponse;
use crate::EffectiveMembership;
use crate::Entry;
use crate::EntryPayload;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;
use crate::Vote;

crate::declare_raft_types!(
    pub(crate) Foo: D=(), R=(), NodeId=u64
);

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn blank(term: u64, index: u64) -> Entry<Foo> {
    Entry {
        log_id: log_id(term, index),
        payload: EntryPayload::<Foo>::Blank,
    }
}

fn m01() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {2,3}], None)
}

fn m34() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {3,4}], None)
}

fn eng() -> Engine<u64> {
    let mut eng = Engine::<u64> {
        id: 2, // make it a member
        ..Default::default()
    };
    eng.state.last_applied = Some(log_id(0, 0));
    eng.state.vote = Vote::new(2, 1);
    eng.state.server_state = ServerState::Candidate;
    eng.state.log_ids.append(log_id(1, 1));
    eng.state.log_ids.append(log_id(2, 3));
    eng.state.committed = Some(log_id(0, 0));
    eng.state.membership_state.committed = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()));
    eng
}

#[test]
fn test_handle_append_entries_req_vote_is_rejected() -> anyhow::Result<()> {
    let mut eng = eng();

    let resp = eng.handle_append_entries_req(&Vote::new(1, 1), None, &Vec::<Entry<Foo>>::new(), None);

    assert_eq!(AppendEntriesResponse::HigherVote(Vote::new(2, 1)), resp);
    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new(2, 1), eng.state.vote);
    assert_eq!(Some(log_id(2, 3)), eng.state.last_log_id());
    assert_eq!(Some(log_id(0, 0)), eng.state.committed);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );
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
fn test_handle_append_entries_req_prev_log_id_conflict() -> anyhow::Result<()> {
    let mut eng = eng();

    let resp = eng.handle_append_entries_req(
        &Vote::new_committed(2, 1),
        Some(log_id(2, 2)),
        &Vec::<Entry<Foo>>::new(),
        None,
    );

    assert_eq!(AppendEntriesResponse::Conflict, resp);
    assert_eq!(
        &[
            log_id(1, 1), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), eng.state.vote);
    assert_eq!(Some(log_id(1, 1)), eng.state.last_log_id());
    assert_eq!(Some(log_id(0, 0)), eng.state.committed);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
        },
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::InstallElectionTimer { can_be_leader: false },
            Command::RejectElection {},
            Command::DeleteConflictLog { since: log_id(1, 2) },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
            },
            Command::UpdateServerState {
                server_state: ServerState::Learner
            },
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_handle_append_entries_req_prev_log_id_is_committed() -> anyhow::Result<()> {
    let mut eng = eng();

    let resp = eng.handle_append_entries_req(
        &Vote::new_committed(2, 1),
        Some(log_id(0, 0)),
        &[blank(1, 1), blank(2, 2)],
        Some(log_id(1, 1)),
    );

    assert_eq!(AppendEntriesResponse::Success, resp);
    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 2), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), eng.state.vote);
    assert_eq!(Some(log_id(2, 2)), eng.state.last_log_id());
    assert_eq!(Some(log_id(1, 1)), eng.state.committed);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
        },
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::InstallElectionTimer { can_be_leader: false },
            Command::RejectElection {},
            Command::DeleteConflictLog { since: log_id(1, 2) },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
            },
            Command::UpdateServerState {
                server_state: ServerState::Learner
            },
            Command::AppendInputEntries { range: 1..2 },
            Command::MoveInputCursorBy { n: 2 },
            Command::FollowerCommit {
                since: Some(log_id(0, 0)),
                upto: log_id(1, 1)
            },
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_handle_append_entries_req_prev_log_id_not_exists() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.vote = Vote::new(1, 2);
    eng.enter_leading();

    let resp = eng.handle_append_entries_req(
        &Vote::new_committed(2, 1),
        Some(log_id(2, 4)),
        &[blank(1, 1), blank(2, 2)],
        Some(log_id(1, 1)),
    );

    assert_eq!(AppendEntriesResponse::Conflict, resp);
    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), eng.state.vote);
    assert_eq!(Some(log_id(2, 3)), eng.state.last_log_id());
    assert_eq!(Some(log_id(0, 0)), eng.state.committed);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );
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
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::InstallElectionTimer { can_be_leader: false },
            Command::RejectElection {},
            Command::UpdateServerState {
                server_state: ServerState::Follower
            },
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_handle_append_entries_req_entries_conflict() -> anyhow::Result<()> {
    // prev_log_id matches,
    // The second entry in entries conflict.
    // This request will replace the effective membership.
    // committed is greater than entries.
    // It is no longer a member, change to learner
    let mut eng = eng();

    let resp = eng.handle_append_entries_req(
        &Vote::new_committed(2, 1),
        Some(log_id(1, 1)),
        &[blank(1, 2), Entry {
            log_id: log_id(3, 3),
            payload: EntryPayload::Membership(m34()),
        }],
        Some(log_id(4, 4)),
    );

    assert_eq!(AppendEntriesResponse::Success, resp);
    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(3, 3), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), eng.state.vote);
    assert_eq!(Some(log_id(3, 3)), eng.state.last_log_id());
    assert_eq!(Some(log_id(3, 3)), eng.state.committed);
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(3, 3)), m34())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(3, 3)), m34()))
        },
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::InstallElectionTimer { can_be_leader: false },
            Command::RejectElection {},
            Command::DeleteConflictLog { since: log_id(2, 3) },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
            },
            Command::UpdateServerState {
                server_state: ServerState::Learner
            },
            Command::AppendInputEntries { range: 1..2 },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 3)), m34()))
            },
            Command::MoveInputCursorBy { n: 2 },
            Command::FollowerCommit {
                since: Some(log_id(0, 0)),
                upto: log_id(3, 3)
            },
        ],
        eng.commands
    );

    Ok(())
}
