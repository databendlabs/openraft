use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::EffectiveMembership;
use crate::Entry;
use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::entry::RaftEntry;
use crate::errors::ConflictingLogId;
use crate::errors::RejectAppendEntries;
use crate::errors::RejectLeadership;
use crate::raft::message::LogSegment;
use crate::raft::message::MatchedLogId;
use crate::raft_state::IOId;
use crate::raft_state::LogStateReader;
use crate::testing::blank_ent;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;
use crate::vote::Leadership;
use crate::vote::raft_vote::RaftVoteExt;

fn m01() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m23() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {2,3}], [])
}

fn m34() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {3,4}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(2, 1));
    // Last-per-leader format: leader 1's last at index 2, leader 2's last at index 3
    eng.state.log_ids.append(log_id(1, 1, 2));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.apply_progress_mut().accept(log_id(0, 1, 0));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng.state.server_state = eng.calc_server_state();
    eng
}

fn leadership(vote: Vote<UTConfig>) -> Leadership<UTConfig> {
    Leadership {
        vote,
        last_log_id: None,
    }
}

fn segment(
    prev_log_id: Option<crate::type_config::alias::LogIdOf<UTConfig>>,
    entries: Vec<Entry<UTConfig>>,
) -> LogSegment<UTConfig> {
    LogSegment { prev_log_id, entries }
}

#[test]
fn test_append_entries_vote_is_rejected() -> anyhow::Result<()> {
    let mut eng = eng();

    let res = eng.append_entries(leadership(Vote::new(1, 1)), segment(None, vec![]));

    assert_eq!(
        Err(RejectAppendEntries::RejectLeadership(RejectLeadership::ByVote(
            Vote::new(2, 1)
        ))),
        res
    );
    assert_eq!(None, eng.state.log_ids.purged());
    assert_eq!(
        &[
            log_id(1, 1, 2), // Last-per-leader: leader 1's last at index 2
            log_id(2, 1, 3),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 1, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_append_entries_prev_log_id_is_applied() -> anyhow::Result<()> {
    // An applied log id has to be committed thus
    let mut eng = eng();
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(1, 2));
    eng.output.take_commands();

    let res = eng.append_entries(
        leadership(Vote::new_committed(2, 1)),
        segment(Some(log_id(0, 1, 0)), vec![]),
    );

    assert_eq!(
        Ok(MatchedLogId {
            log_id: Some(log_id(0, 1, 0))
        }),
        res
    );
    assert_eq!(None, eng.state.log_ids.purged());
    assert_eq!(
        &[
            log_id(1, 1, 2), // Last-per-leader: leader 1's last at index 2
            log_id(2, 1, 3), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 1, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::CloseReplicationStreams,
            Command::UpdateIOProgress {
                when: Some(Condition::IOFlushed {
                    io_id: IOId::new_log_io(Vote::new(2, 1).into_committed(), None)
                }),
                io_id: IOId::new_log_io(Vote::new(2, 1).into_committed(), Some(log_id(0, 1, 0)))
            }
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_append_entries_prev_log_id_conflict() -> anyhow::Result<()> {
    let mut eng = eng();

    let res = eng.append_entries(
        leadership(Vote::new_committed(2, 1)),
        segment(Some(log_id(2, 1, 2)), vec![]),
    );

    assert_eq!(
        Err(RejectAppendEntries::ConflictingLogId(ConflictingLogId {
            expect: log_id(2, 1, 2),
            local: Some(log_id(1, 1, 2)),
        })),
        res
    );
    assert_eq!(None, eng.state.log_ids.purged());
    assert_eq!(
        &[
            log_id(1, 1, 1), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(1, 1, 1)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::CloseReplicationStreams,
            Command::TruncateLog {
                after: Some(log_id(1, 1, 1))
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_append_entries_prev_log_id_is_committed() -> anyhow::Result<()> {
    let mut eng = eng();

    let res = eng.append_entries(
        leadership(Vote::new_committed(2, 1)),
        segment(Some(log_id(0, 1, 0)), vec![blank_ent(1, 1, 1), blank_ent(2, 1, 2)]),
    );

    assert_eq!(
        Ok(MatchedLogId {
            log_id: Some(log_id(2, 1, 2))
        }),
        res
    );
    assert_eq!(None, eng.state.log_ids.purged());
    assert_eq!(
        &[
            log_id(1, 1, 1), // After truncation: leader 1 ends at index 1
            log_id(2, 1, 2), // New leader 2 entry at index 2
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 1, 2)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::CloseReplicationStreams,
            Command::TruncateLog {
                after: Some(log_id(1, 1, 1))
            },
            Command::AppendEntries {
                committed_vote: Vote::new(2, 1).into_committed(),
                entries: [blank_ent(2, 1, 2)].into()
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_append_entries_prev_log_id_not_exists() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(1, 2));
    eng.output.take_commands();

    let res = eng.append_entries(
        leadership(Vote::new_committed(2, 1)),
        segment(Some(log_id(2, 1, 4)), vec![blank_ent(2, 1, 5), blank_ent(2, 1, 6)]),
    );

    assert_eq!(
        Err(RejectAppendEntries::ConflictingLogId(ConflictingLogId {
            expect: log_id(2, 1, 4),
            local: None,
        })),
        res
    );
    assert_eq!(None, eng.state.log_ids.purged());
    assert_eq!(
        &[
            log_id(1, 1, 2), // Last-per-leader: leader 1's last at index 2
            log_id(2, 1, 3), //
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(2, 1, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::CloseReplicationStreams,
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_append_entries_conflict() -> anyhow::Result<()> {
    // prev_log_id matches,
    // The second entry in entries conflict.
    // This request will replace the effective membership.
    // committed is greater than entries.
    // It is no longer a member, change to learner
    let mut eng = eng();

    let resp = eng.append_entries(
        leadership(Vote::new_committed(2, 1)),
        segment(Some(log_id(1, 1, 1)), vec![
            blank_ent(1, 1, 2),
            Entry::new_membership(log_id(3, 1, 3), m34()),
        ]),
    );

    assert_eq!(
        Ok(MatchedLogId {
            log_id: Some(log_id(3, 1, 3))
        }),
        resp
    );
    assert_eq!(None, eng.state.log_ids.purged());
    assert_eq!(
        &[
            log_id(1, 1, 2), // Leader 1's last is at index 2 (blank_ent at index 2)
            log_id(3, 1, 3), // Leader 3's last is at index 3 (membership entry)
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
    assert_eq!(Some(&log_id(3, 1, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 3)), m34())),
        ),
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new_committed(2, 1)
            },
            Command::CloseReplicationStreams,
            Command::TruncateLog {
                after: Some(log_id(1, 1, 2))
            },
            Command::AppendEntries {
                committed_vote: Vote::new(2, 1).into_committed(),
                entries: [Entry::new_membership(log_id(3, 1, 3), m34())].into()
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
