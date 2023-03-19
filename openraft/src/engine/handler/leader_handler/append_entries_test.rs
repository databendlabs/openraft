use std::sync::Arc;

use maplit::btreeset;
#[allow(unused_imports)] use pretty_assertions::assert_eq;
#[allow(unused_imports)] use pretty_assertions::assert_ne;
#[allow(unused_imports)] use pretty_assertions::assert_str_eq;
use tokio::time::Instant;

use crate::engine::Command;
use crate::engine::Engine;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::raft_state::LogStateReader;
use crate::utime::UTime;
use crate::vote::CommittedLeaderId;
use crate::EffectiveMembership;
use crate::Entry;
use crate::EntryPayload;
use crate::LogId;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;
use crate::ServerState;
use crate::Vote;

crate::declare_raft_types!(
    pub(crate) Foo: D=(), R=(), NodeId=u64, Node=(), Entry = crate::Entry<Foo>
);

use crate::testing::log_id;

fn blank(term: u64, index: u64) -> Entry<Foo> {
    Entry {
        log_id: log_id(term, index),
        payload: EntryPayload::<Foo>::Blank,
    }
}

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

fn m13() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,3}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {2,3}], None)
}

fn m34() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {3,4}], None)
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::default();
    eng.state.enable_validate = false; // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.committed = Some(log_id(0, 0));
    eng.state.vote = UTime::new(Instant::now(), Vote::new_committed(3, 1));
    eng.state.log_ids.append(log_id(1, 1));
    eng.state.log_ids.append(log_id(2, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
    );
    eng.state.server_state = eng.calc_server_state();

    eng
}

#[test]
fn test_leader_append_entries_empty() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.vote_handler().become_leading();

    eng.leader_handler()?.leader_append_entries(&mut Vec::<Entry<Foo>>::new());

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(Some(&log_id(2, 3)), eng.state.last_log_id());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );

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
fn test_leader_append_entries_normal() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.vote_handler().become_leading();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(&mut [
        blank(1, 1), //
        blank(1, 1),
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
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
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            Command::Replicate {
                target: 2,
                req: Inflight::logs(None, Some(log_id(3, 6))).with_id(1),
            },
            Command::Replicate {
                target: 3,
                req: Inflight::logs(None, Some(log_id(3, 6))).with_id(1),
            },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.output.commands
    );

    Ok(())
}

#[test]
fn test_leader_append_entries_fast_commit() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1())));
    eng.vote_handler().become_leading();

    eng.output.commands = vec![];
    eng.output.metrics_flags.reset();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(&mut [
        blank(1, 1), //
        blank(1, 1),
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
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
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1())),
        ),
        eng.state.membership_state
    );
    assert_eq!(
        Some(&LogId::new(CommittedLeaderId::new(3, 1), 6)),
        eng.state.committed()
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            Command::ReplicateCommitted {
                committed: Some(log_id(3, 6))
            },
            Command::LeaderCommit {
                already_committed: Some(log_id(0, 0)),
                upto: LogId::new(CommittedLeaderId::new(3, 1), 6)
            },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.output.commands
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    Ok(())
}

/// With membership log, fast-commit upto membership entry that is not a single-node
/// cluster. Leader is no longer a voter should work.
#[test]
fn test_leader_append_entries_fast_commit_upto_membership_entry() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1())));
    eng.state.server_state = ServerState::Leader;
    eng.vote_handler().become_leading();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(&mut [
        blank(1, 1), //
        Entry {
            log_id: log_id(1, 1),
            payload: EntryPayload::Membership(m34()),
        },
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
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
            // previous effective become committed.
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1())),
            // new effective.
            Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m34())),
        ),
        eng.state.membership_state
    );
    assert_eq!(
        Some(&LogId::new(CommittedLeaderId::new(3, 1), 4)),
        eng.state.committed()
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: true,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            Command::ReplicateCommitted {
                committed: Some(log_id(3, 4))
            },
            Command::LeaderCommit {
                already_committed: Some(log_id(0, 0)),
                upto: LogId::new(CommittedLeaderId::new(3, 1), 4)
            },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(
                    Some(LogId::new(CommittedLeaderId::new(3, 1), 5)),
                    m34()
                )),
            },
            Command::RebuildReplicationStreams {
                targets: vec![(3, ProgressEntry::empty(7)), (4, ProgressEntry::empty(7))]
            },
            Command::Replicate {
                target: 3,
                req: Inflight::logs(None, Some(log_id(3, 6))).with_id(1),
            },
            Command::Replicate {
                target: 4,
                req: Inflight::logs(None, Some(log_id(3, 6))).with_id(1),
            },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.output.commands
    );

    Ok(())
}

/// With membership log, fast-commit is allowed if no voter change.
#[test]
fn test_leader_append_entries_fast_commit_membership_no_voter_change() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m1())));
    eng.vote_handler().become_leading();
    eng.state.server_state = eng.calc_server_state();

    eng.output.commands = vec![];
    eng.output.metrics_flags.reset();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(&mut [
        blank(1, 1), //
        Entry {
            log_id: log_id(1, 1),
            payload: EntryPayload::Membership(m1_2()),
        },
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
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
            Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m1_2())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m1_2())),
        ),
        eng.state.membership_state
    );
    assert_eq!(
        Some(&LogId::new(CommittedLeaderId::new(3, 1), 6)),
        eng.state.committed()
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: true,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            // first commit upto the membership entry(exclusive).
            Command::ReplicateCommitted {
                committed: Some(log_id(3, 4))
            },
            Command::LeaderCommit {
                already_committed: Some(log_id(0, 0)),
                upto: LogId::new(CommittedLeaderId::new(3, 1), 4)
            },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(
                    Some(LogId::new(CommittedLeaderId::new(3, 1), 5)),
                    m1_2()
                )),
            },
            Command::RebuildReplicationStreams {
                targets: vec![(2, ProgressEntry::empty(7))]
            },
            Command::Replicate {
                target: 2,
                req: Inflight::logs(None, Some(log_id(3, 6))).with_id(1),
            },
            // second commit upto the end.
            Command::ReplicateCommitted {
                committed: Some(log_id(3, 6))
            },
            Command::LeaderCommit {
                already_committed: Some(LogId::new(CommittedLeaderId::new(3, 1), 4)),
                upto: LogId::new(CommittedLeaderId::new(3, 1), 6)
            },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.output.commands
    );

    Ok(())
}

// TODO(xp): check progress

/// With membership log, fast-commit all if the membership log changes to one voter.
#[test]
fn test_leader_append_entries_fast_commit_if_membership_voter_change_to_1() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m13())));
    eng.vote_handler().become_leading();
    eng.state.server_state = eng.calc_server_state();

    eng.output.commands = vec![];
    eng.output.metrics_flags.reset();

    // log id will be assigned by eng.
    eng.leader_handler()?.leader_append_entries(&mut [
        blank(1, 1), //
        Entry {
            log_id: log_id(1, 1),
            payload: EntryPayload::Membership(m1_2()),
        },
        blank(1, 1),
    ]);

    assert_eq!(
        &[
            log_id(1, 1), //
            log_id(2, 3),
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
            Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m1_2())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 5)), m1_2())),
        ),
        eng.state.membership_state
    );
    assert_eq!(
        Some(&LogId::new(CommittedLeaderId::new(3, 1), 6)),
        eng.state.committed()
    );

    assert_eq!(
        vec![
            Command::AppendInputEntries { range: 0..3 },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(
                    Some(LogId::new(CommittedLeaderId::new(3, 1), 5)),
                    m1_2()
                )),
            },
            Command::RebuildReplicationStreams {
                targets: vec![(2, ProgressEntry::empty(7))]
            },
            Command::Replicate {
                target: 2,
                req: Inflight::logs(None, Some(log_id(3, 6))).with_id(1),
            },
            // It is correct to commit if the membership change to a one node cluster.
            Command::ReplicateCommitted {
                committed: Some(log_id(3, 6))
            },
            Command::LeaderCommit {
                already_committed: Some(log_id(0, 0)),
                upto: LogId::new(CommittedLeaderId::new(3, 1), 6)
            },
            Command::MoveInputCursorBy { n: 3 },
        ],
        eng.output.commands
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: true,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    Ok(())
}
