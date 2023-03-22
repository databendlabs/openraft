use std::sync::Arc;

use maplit::btreeset;
use pretty_assertions::assert_eq;
use tokio::time::Instant;

use crate::core::ServerState;
use crate::engine::testing::UTCfg;
use crate::engine::CEngine;
use crate::engine::Command;
use crate::engine::Engine;
use crate::raft::AppendEntriesResponse;
use crate::raft_state::LogStateReader;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::EffectiveMembership;
use crate::Entry;
use crate::EntryPayload;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;
use crate::Vote;

fn blank(term: u64, index: u64) -> Entry<UTCfg> {
    Entry {
        log_id: log_id(term, index),
        payload: EntryPayload::<UTCfg>::Blank,
    }
}

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {2,3}], None)
}

fn m34() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {3,4}], None)
}

fn eng() -> CEngine<UTCfg> {
    let mut eng = Engine::default();
    eng.state.enable_validate = false; // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.vote = UTime::new(Instant::now(), Vote::new(2, 1));
    eng.state.log_ids.append(log_id(1, 1));
    eng.state.log_ids.append(log_id(2, 3));
    eng.state.committed = Some(log_id(0, 0));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
    );
    eng.state.server_state = eng.calc_server_state();
    eng
}

#[test]
fn test_handle_append_entries_req_vote_is_rejected() -> anyhow::Result<()> {
    let mut eng = eng();

    let resp = eng.handle_append_entries_req(&Vote::new(1, 1), None, &Vec::<Entry<UTCfg>>::new(), None);

    assert_eq!(AppendEntriesResponse::HigherVote(Vote::new(2, 1)), resp);
    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 3)), eng.state.last_log_id());
    assert_eq!(Some(&log_id(0, 0)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(0, eng.output.commands.len());

    Ok(())
}

#[test]
fn test_handle_append_entries_req_prev_log_id_is_applied() -> anyhow::Result<()> {
    // An applied log id has to be committed thus
    let mut eng = eng();
    eng.state.vote = UTime::new(Instant::now(), Vote::new(1, 2));
    eng.vote_handler().become_leading();

    let resp = eng.handle_append_entries_req(
        &Vote::new_committed(2, 1),
        Some(log_id(0, 0)),
        &Vec::<Entry<UTCfg>>::new(),
        None,
    );

    assert_eq!(AppendEntriesResponse::Success, resp);
    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 3)), eng.state.last_log_id());
    assert_eq!(Some(&log_id(0, 0)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![Command::SaveVote {
            vote: Vote::new_committed(2, 1)
        },],
        eng.output.commands
    );

    Ok(())
}

#[test]
fn test_handle_append_entries_req_prev_log_id_conflict() -> anyhow::Result<()> {
    let mut eng = eng();

    let resp = eng.handle_append_entries_req(
        &Vote::new_committed(2, 1),
        Some(log_id(2, 2)),
        &Vec::<Entry<UTCfg>>::new(),
        None,
    );

    assert_eq!(AppendEntriesResponse::Conflict, resp);
    assert_eq!(
        &[
            log_id(1, 1), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(1, 1)), eng.state.last_log_id());
    assert_eq!(Some(&log_id(0, 0)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::DeleteConflictLog { since: log_id(1, 2) },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
            },
        ],
        eng.output.commands
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
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 2)), eng.state.last_log_id());
    assert_eq!(Some(&log_id(1, 1)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::DeleteConflictLog { since: log_id(1, 2) },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
            },
            Command::AppendInputEntries { range: 1..2 },
            Command::FollowerCommit {
                already_committed: Some(log_id(0, 0)),
                upto: log_id(1, 1)
            },
        ],
        eng.output.commands
    );

    Ok(())
}

#[test]
fn test_handle_append_entries_req_prev_log_id_not_exists() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.vote = UTime::new(Instant::now(), Vote::new(1, 2));
    eng.vote_handler().become_leading();

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
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 3)), eng.state.last_log_id());
    assert_eq!(Some(&log_id(0, 0)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![Command::SaveVote {
            vote: Vote::new_committed(2, 1)
        },],
        eng.output.commands
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
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(3, 3)), eng.state.last_log_id());
    assert_eq!(Some(&log_id(3, 3)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(3, 3)), m34())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 3)), m34())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::DeleteConflictLog { since: log_id(2, 3) },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
            },
            Command::AppendInputEntries { range: 1..2 },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 3)), m34()))
            },
            Command::FollowerCommit {
                already_committed: Some(log_id(0, 0)),
                upto: log_id(3, 3)
            },
        ],
        eng.output.commands
    );

    Ok(())
}
