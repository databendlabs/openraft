use std::sync::Arc;

use maplit::btreeset;
#[allow(unused_imports)] use pretty_assertions::assert_eq;
#[allow(unused_imports)] use pretty_assertions::assert_ne;
#[allow(unused_imports)] use pretty_assertions::assert_str_eq;

use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::entry::RaftEntry;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::raft_state::LogStateReader;
use crate::testing::blank_ent;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::vote::CommittedLeaderId;
use crate::EffectiveMembership;
use crate::Entry;
use crate::LogId;
use crate::Membership;
use crate::MembershipState;
use crate::TokioInstant;
use crate::Vote;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

fn m1() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1}], None)
}

/// members: {1}, learners: {2}
fn m1_2() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1}], Some(btreeset! {2}))
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {2,3}], btreeset! {1,2,3})
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.committed = Some(log_id(0, 1, 0));
    eng.state.vote = UTime::new(TokioInstant::now(), Vote::new_committed(3, 1));
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng.state.server_state = eng.calc_server_state();

    eng
}

#[test]
fn test_leader_append_entries_empty() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.vote_handler().become_leading();

    eng.leader_handler()?.leader_append_entries(Vec::<Entry<UTConfig>>::new());

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
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_leader_append_entries_normal() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.vote_handler().become_leading();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(vec![
        blank_ent(1, 1, 1), //
        blank_ent(1, 1, 1),
        blank_ent(1, 1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
            LogId::new(CommittedLeaderId::new(3, 1), 4),
            LogId::new(CommittedLeaderId::new(3, 1), 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(
        Some(&LogId::new(CommittedLeaderId::new(3, 1), 6)),
        eng.state.last_log_id()
    );
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        eng.state.membership_state
    );
    assert_eq!(
        vec![
            Command::AppendInputEntries {
                entries: vec![
                    blank_ent(3, 1, 4), //
                    blank_ent(3, 1, 5),
                    blank_ent(3, 1, 6),
                ]
            },
            Command::Replicate {
                target: 2,
                req: Inflight::logs(None, Some(log_id(3, 1, 6))).with_id(1),
            },
            Command::Replicate {
                target: 3,
                req: Inflight::logs(None, Some(log_id(3, 1, 6))).with_id(1),
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
    eng.vote_handler().become_leading();

    eng.output.clear_commands();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(vec![
        blank_ent(1, 1, 1), //
        blank_ent(1, 1, 1),
        blank_ent(1, 1, 1),
    ]);

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
        vec![Command::AppendInputEntries {
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
    eng.vote_handler().become_leading();
    eng.state.server_state = eng.calc_server_state();

    eng.output.clear_commands();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(vec![
        blank_ent(1, 1, 1), //
        Entry::new_membership(log_id(1, 1, 1), m1_2()),
        blank_ent(1, 1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
            log_id(3, 1, 4),
            log_id(3, 1, 6),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(
        Some(&LogId::new(CommittedLeaderId::new(3, 1), 6)),
        eng.state.last_log_id()
    );
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
            Command::AppendInputEntries {
                entries: vec![
                    blank_ent(3, 1, 4), //
                    Entry::new_membership(log_id(3, 1, 5), m1_2()),
                    blank_ent(3, 1, 6),
                ]
            },
            Command::RebuildReplicationStreams {
                targets: vec![(2, ProgressEntry::empty(7))]
            },
            Command::Replicate {
                target: 2,
                req: Inflight::logs(None, Some(log_id(3, 1, 6))).with_id(1),
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
