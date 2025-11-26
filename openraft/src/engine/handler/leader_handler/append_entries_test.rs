use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
#[allow(unused_imports)]
use pretty_assertions::assert_eq;
#[allow(unused_imports)]
use pretty_assertions::assert_ne;
#[allow(unused_imports)]
use pretty_assertions::assert_str_eq;

use crate::EffectiveMembership;
use crate::Entry;
use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::ReplicationProgress;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::entry::RaftEntry;
use crate::log_id_range::LogIdRange;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::raft_state::IOId;
use crate::raft_state::LogStateReader;
use crate::replication::request::Replicate;
use crate::testing::blank_ent;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

fn m01() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m1() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1}], [])
}

/// members: {1}, learners: {2}
fn m1_2() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1}], btreeset! {2})
}

fn m23() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {2,3}], btreeset! {1,2,3})
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.apply_progress_mut().accept(log_id(0, 1, 0));
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 1),
    );
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng.testing_new_leader();
    eng.state.server_state = eng.calc_server_state();

    eng
}

#[test]
fn test_leader_append_entries_empty() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.output.take_commands();

    eng.leader_handler()?.leader_append_entries(Vec::<Entry<UTConfig>>::new());

    assert_eq!(
        None,
        eng.state.accepted_log_io(),
        "no accepted log updated for empty entries"
    );
    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(2, 1, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(eng.output.take_commands(), vec![]);

    Ok(())
}

#[test]
fn test_leader_append_entries_normal() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.output.take_commands();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(vec![
        blank_ent(1, 1, 1), //
        blank_ent(1, 1, 1),
        blank_ent(1, 1, 1),
    ]);

    assert_eq!(
        Some(&IOId::new_log_io(
            Vote::new(3, 1).into_committed(),
            Some(log_id(3, 1, 6))
        )),
        eng.state.accepted_log_io()
    );
    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
            log_id(3, 1, 4),
            log_id(3, 1, 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(3, 1, 6)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(
        vec![
            Command::AppendEntries {
                committed_vote: Vote::new(3, 1).into_committed(),
                entries: vec![
                    blank_ent(3, 1, 4), //
                    blank_ent(3, 1, 5),
                    blank_ent(3, 1, 6),
                ]
            },
            Command::Replicate {
                target: 2,
                req: Replicate::logs(LogIdRange::new(None, Some(log_id(3, 1, 6))), InflightId::new(1)),
            },
            Command::Replicate {
                target: 3,
                req: Replicate::logs(LogIdRange::new(None, Some(log_id(3, 1, 6))), InflightId::new(2)),
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_leader_append_entries_single_node_leader() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m1())));
    eng.testing_new_leader();

    eng.output.clear_commands();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(vec![
        blank_ent(1, 1, 1), //
        blank_ent(1, 1, 1),
        blank_ent(1, 1, 1),
    ]);

    assert_eq!(
        Some(&IOId::new_log_io(
            Vote::new(3, 1).into_committed(),
            Some(log_id(3, 1, 6))
        )),
        eng.state.accepted_log_io()
    );
    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
            log_id(3, 1, 4),
            log_id(3, 1, 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(3, 1, 6)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m1())),
        ),
        eng.state.membership_state
    );
    assert_eq!(Some(&log_id(0, 1, 0)), eng.state.committed());

    assert_eq!(
        vec![Command::AppendEntries {
            committed_vote: Vote::new(3, 1).into_committed(),
            entries: vec![
                blank_ent(3, 1, 4), //
                blank_ent(3, 1, 5),
                blank_ent(3, 1, 6),
            ]
        },],
        eng.output.take_commands()
    );
    Ok(())
}

#[test]
fn test_leader_append_entries_with_membership_log() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m1())));
    eng.testing_new_leader();
    eng.state.server_state = eng.calc_server_state();

    eng.output.clear_commands();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(vec![
        blank_ent(1, 1, 1), //
        Entry::new_membership(log_id(1, 1, 1), m1_2()),
        blank_ent(1, 1, 1),
    ]);

    assert_eq!(
        Some(&IOId::new_log_io(
            Vote::new(3, 1).into_committed(),
            Some(log_id(3, 1, 6))
        )),
        eng.state.accepted_log_io()
    );
    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
            log_id(3, 1, 4),
            log_id(3, 1, 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(3, 1, 6)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m1())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 5)), m1_2())),
        ),
        eng.state.membership_state
    );
    assert_eq!(Some(&log_id(0, 1, 0)), eng.state.committed());

    assert_eq!(
        vec![
            Command::AppendEntries {
                committed_vote: Vote::new(3, 1).into_committed(),
                entries: vec![
                    blank_ent(3, 1, 4), //
                    Entry::new_membership(log_id(3, 1, 5), m1_2()),
                    blank_ent(3, 1, 6),
                ]
            },
            Command::RebuildReplicationStreams {
                targets: vec![ReplicationProgress(2, ProgressEntry::empty(7))]
            },
            Command::Replicate {
                target: 2,
                req: Replicate::logs(LogIdRange::new(None, Some(log_id(3, 1, 6))), InflightId::new(1))
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
