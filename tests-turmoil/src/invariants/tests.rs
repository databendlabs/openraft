//! Unit tests for the invariant checkers.
//!
//! Each test constructs a handful of [`FullNodeSnapshot`]s by hand, feeds them
//! to the relevant checker, and asserts which violation variants (if any)
//! pop out. Using the public [`InvariantChecker::check`] entry point keeps the
//! tests close to the real call site, while the helpers keep the fixtures
//! readable.

use std::sync::Arc;

use openraft::EntryPayload;
use openraft::alias::LogIdListOf;
use openraft::metrics::RaftMetrics;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use openraft::type_config::TypeConfigExt;
use openraft::vote::RaftLeaderId;

use super::CLeaderId;
use super::DurableLogIds;
use super::InvariantChecker;
use super::InvariantViolation;
use crate::cluster::ClusterState;
use crate::cluster::FullNodeSnapshot;
use crate::storage_model::StorageModel;
use crate::store::LogStore;
use crate::store::StateMachineData;
use crate::typ::Entry;
use crate::typ::LogId;
use crate::typ::NodeId;
use crate::typ::StoredMembership;
use crate::typ::TypeConfig;
use crate::typ::Vote;

// --- Fixture builders ---------------------------------------------------

fn clid(term: u64, node_id: NodeId) -> CLeaderId {
    openraft::impls::leader_id_adv::LeaderId::new(term, node_id)
}

fn log_id(term: u64, node_id: NodeId, index: u64) -> LogId {
    LogId::new(clid(term, node_id), index)
}

/// A mutable builder that defaults every snapshot field to a safe "empty"
/// value and lets individual tests override just the ones they care about.
struct SnapshotBuilder {
    node_id: NodeId,
    term: u64,
    vote_leader: (u64, NodeId),
    vote_committed: bool,
    state: openraft::ServerState,
    last_log_index: Option<u64>,
    committed: Option<LogId>,
    last_applied: Option<LogId>,
    snapshot: Option<LogId>,
    purged: Option<LogId>,
    log_entries: Vec<LogId>,
    membership_log_id: Option<LogId>,
    membership: Vec<Vec<NodeId>>,
    sm_last_applied: Option<LogId>,
    sm_data: std::collections::BTreeMap<String, crate::store::ValueMeta>,
}

impl SnapshotBuilder {
    fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            term: 0,
            vote_leader: (0, node_id),
            vote_committed: false,
            state: openraft::ServerState::Follower,
            last_log_index: None,
            committed: None,
            last_applied: None,
            snapshot: None,
            purged: None,
            log_entries: Vec::new(),
            membership_log_id: None,
            membership: Vec::new(),
            sm_last_applied: None,
            sm_data: std::collections::BTreeMap::new(),
        }
    }

    fn term(mut self, t: u64) -> Self {
        self.term = t;
        self
    }

    fn leader(mut self, leader_term: u64, leader_id: NodeId, committed: bool) -> Self {
        self.vote_leader = (leader_term, leader_id);
        self.vote_committed = committed;
        self
    }

    fn state(mut self, s: openraft::ServerState) -> Self {
        self.state = s;
        self
    }

    fn committed(mut self, lid: LogId) -> Self {
        self.committed = Some(lid);
        self.last_log_index = Some(lid.index().max(self.last_log_index.unwrap_or(0)));
        self
    }

    fn applied(mut self, lid: LogId) -> Self {
        self.last_applied = Some(lid);
        self.sm_last_applied = Some(lid);
        self
    }

    fn snapshot(mut self, lid: LogId) -> Self {
        self.snapshot = Some(lid);
        self.last_log_index = Some(lid.index().max(self.last_log_index.unwrap_or(0)));
        self
    }

    fn purged(mut self, lid: LogId) -> Self {
        self.purged = Some(lid);
        self
    }

    fn log_entries(mut self, entries: impl IntoIterator<Item = LogId>) -> Self {
        self.log_entries = entries.into_iter().collect();
        if let Some(last) = self.log_entries.last() {
            self.last_log_index = Some(last.index());
        }
        self
    }

    fn membership_voters(mut self, voter_sets: Vec<Vec<NodeId>>) -> Self {
        self.membership = voter_sets;
        self
    }

    fn membership_log_id(mut self, lid: LogId) -> Self {
        self.membership_log_id = Some(lid);
        self
    }

    fn sm_data(mut self, k: &str, v: &str) -> Self {
        self.sm_data.insert(k.into(), crate::store::ValueMeta {
            value: v.into(),
            serial: 0,
            log_id: log_id(1, 1, 1),
        });
        self
    }

    fn sm_key_at(mut self, k: &str, lid: LogId) -> Self {
        self.sm_data.insert(k.into(), crate::store::ValueMeta {
            value: format!("v-{k}"),
            serial: 0,
            log_id: lid,
        });
        self
    }

    fn build(self) -> (NodeId, FullNodeSnapshot) {
        let mut raft = RaftMetrics::<TypeConfig>::new_initial(self.node_id);
        raft.current_term = self.term;
        let mut vote = Vote::new(self.vote_leader.0, self.vote_leader.1);
        if self.vote_committed {
            vote = Vote::new_committed(self.vote_leader.0, self.vote_leader.1);
        }
        raft.vote = vote;
        raft.state = self.state;
        raft.last_log_index = self.last_log_index;
        raft.local_committed = self.committed;
        raft.last_applied = self.last_applied;
        raft.snapshot = self.snapshot;
        raft.purged = self.purged;
        raft.log_id_list = LogIdListOf::<TypeConfig>::new(self.purged, self.log_entries);

        if !self.membership.is_empty() {
            let sets: Vec<std::collections::BTreeSet<NodeId>> =
                self.membership.into_iter().map(|v| v.into_iter().collect()).collect();
            let mem = crate::typ::majority_membership(sets, Vec::new());
            raft.membership_config = Arc::new(StoredMembership::new(self.membership_log_id, mem));
        }

        let sm = StateMachineData {
            last_applied: self.sm_last_applied,
            last_membership: StoredMembership::default(),
            data: self.sm_data,
        };

        (self.node_id, FullNodeSnapshot { raft, sm })
    }
}

fn check(snapshots: &[(NodeId, FullNodeSnapshot)]) -> Vec<InvariantViolation> {
    InvariantChecker::default().check(snapshots).violations
}

fn durable_logs(snapshots: &[(NodeId, FullNodeSnapshot)]) -> DurableLogIds {
    snapshots.iter().map(|(id, s)| (*id, s.raft.log_id_list.clone())).collect()
}

fn check_with_durable_logs(
    snapshots: &[(NodeId, FullNodeSnapshot)],
    durable_logs: &DurableLogIds,
) -> Vec<InvariantViolation> {
    InvariantChecker::default().check_with_durable_logs(snapshots, durable_logs).violations
}

// --- Helpers for assertions --------------------------------------------

macro_rules! assert_variant {
    ($violations:expr, $pat:pat) => {
        assert!(
            $violations.iter().any(|v| matches!(v, $pat)),
            "expected violation matching {}, got: {:?}",
            stringify!($pat),
            $violations
        );
    };
}

macro_rules! assert_no_violations {
    ($violations:expr) => {
        assert!(
            $violations.is_empty(),
            "expected no violations, got: {:?}",
            $violations
        );
    };
}

// --- Tests --------------------------------------------------------------

#[test]
fn happy_path_no_violations() {
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(2)
            .leader(2, 1, true)
            .state(openraft::ServerState::Leader)
            .log_entries([log_id(1, 1, 1), log_id(2, 1, 2)])
            .committed(log_id(2, 1, 2))
            .applied(log_id(2, 1, 2))
            .membership_voters(vec![vec![1, 2, 3]])
            .build(),
        SnapshotBuilder::new(2)
            .term(2)
            .leader(2, 1, true)
            .state(openraft::ServerState::Follower)
            .log_entries([log_id(1, 1, 1), log_id(2, 1, 2)])
            .committed(log_id(2, 1, 2))
            .applied(log_id(2, 1, 2))
            .membership_voters(vec![vec![1, 2, 3]])
            .build(),
        SnapshotBuilder::new(3)
            .term(2)
            .leader(2, 1, true)
            .state(openraft::ServerState::Follower)
            .log_entries([log_id(1, 1, 1), log_id(2, 1, 2)])
            .committed(log_id(2, 1, 2))
            .applied(log_id(2, 1, 2))
            .membership_voters(vec![vec![1, 2, 3]])
            .build(),
    ];

    assert_no_violations!(check(&snaps));
}

#[test]
fn election_safety_duplicate_leader() {
    // Two nodes both claim leader with *identical* (term, node_id)
    // — that's a DuplicateLeader even in adv-mode.
    let snaps = vec![
        SnapshotBuilder::new(1).term(5).leader(5, 1, true).state(openraft::ServerState::Leader).build(),
        SnapshotBuilder::new(2)
            .term(5)
            .leader(5, 1, true) // impersonating node 1
            .state(openraft::ServerState::Leader)
            .build(),
    ];

    let violations = check(&snaps);
    assert_variant!(violations, InvariantViolation::DuplicateLeader { .. });
}

#[test]
fn election_safety_adv_mode_same_term_different_node_id_ok() {
    // In adv-mode, (term=5, node_id=1) and (term=5, node_id=2) are legitimate
    // — multiple leaders in the same term, distinguished by node_id.
    let snaps = vec![
        SnapshotBuilder::new(1).term(5).leader(5, 1, true).state(openraft::ServerState::Leader).build(),
        SnapshotBuilder::new(2).term(5).leader(5, 2, true).state(openraft::ServerState::Leader).build(),
    ];

    let violations = check(&snaps);
    assert!(
        !violations.iter().any(|v| matches!(v, InvariantViolation::DuplicateLeader { .. })),
        "adv-mode same-term different-node_id must not be flagged as DuplicateLeader: {violations:?}"
    );
}

#[test]
fn log_matching_committed_divergence() {
    // Both nodes have index 2 committed, but with different leader ids.
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(2)
            .log_entries([log_id(1, 1, 1), log_id(1, 1, 2)])
            .committed(log_id(1, 1, 2))
            .build(),
        SnapshotBuilder::new(2)
            .term(2)
            .log_entries([log_id(1, 1, 1), log_id(2, 2, 2)])
            .committed(log_id(2, 2, 2))
            .build(),
    ];

    let violations = check(&snaps);
    assert_variant!(violations, InvariantViolation::LogDivergence { index: 2, .. });
}

#[test]
fn log_matching_checks_at_purged_boundary() {
    // Regression test for the bug this refactor fixes.
    //
    // Node 1 has purged up to idx 1 with leader id (1,1); node 2 reports a
    // divergent (2,2) at idx 1. The old code started at `purged.index + 1`
    // and missed this — the new code includes the purged entry.
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(2)
            .log_entries([log_id(1, 1, 1)])
            .purged(log_id(1, 1, 1))
            .committed(log_id(1, 1, 1))
            .build(),
        SnapshotBuilder::new(2).term(2).log_entries([log_id(2, 2, 1)]).committed(log_id(2, 2, 1)).build(),
    ];

    let violations = check(&snaps);
    assert_variant!(violations, InvariantViolation::LogDivergence { index: 1, .. });
}

#[test]
fn log_matching_ignores_unwitnessed_snapshot_prefix() {
    let compacted = log_id(1, 1, 10);
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(2)
            .snapshot(compacted)
            .purged(compacted)
            .committed(compacted)
            .applied(compacted)
            .build(),
        SnapshotBuilder::new(2).log_entries([log_id(2, 2, 5), compacted]).committed(compacted).build(),
    ];

    let violations = check(&snaps);
    assert!(
        !violations.iter().any(|v| matches!(v, InvariantViolation::LogDivergence { .. })),
        "unwitnessed snapshot prefix must not be treated as comparable log evidence: {violations:?}"
    );
}

#[test]
fn leader_completeness_violation_missing_committed() {
    // Node 1 was elected with leader_id = (1,1) and committed idx 1.
    // Node 2 is now elected with leader_id = (2,2) but is missing idx 1.
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(1)
            .leader(1, 1, true)
            .log_entries([log_id(1, 1, 1)])
            .committed(log_id(1, 1, 1))
            .build(),
        SnapshotBuilder::new(2)
            .term(2)
            .leader(2, 2, true)
            .state(openraft::ServerState::Leader)
            // Intentionally missing idx 1 in log_entries.
            .build(),
    ];

    let violations = check(&snaps);
    assert_variant!(violations, InvariantViolation::LeaderMissingCommitted {
        missing_index: 1,
        ..
    });
}

#[test]
fn leader_completeness_accepts_snapshot_covered_prefix() {
    let compacted = log_id(2, 2, 10);
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(1)
            .leader(1, 1, true)
            .log_entries([log_id(1, 1, 5)])
            .committed(log_id(1, 1, 5))
            .build(),
        SnapshotBuilder::new(2)
            .term(2)
            .leader(2, 2, true)
            .state(openraft::ServerState::Leader)
            .snapshot(compacted)
            .purged(compacted)
            .committed(compacted)
            .applied(compacted)
            .build(),
    ];

    let violations = check(&snaps);
    assert!(
        !violations.iter().any(|v| matches!(v, InvariantViolation::LeaderMissingCommitted { .. })),
        "leader snapshot-covered prefix must satisfy LeaderCompleteness: {violations:?}"
    );
}

#[test]
fn state_machine_safety_divergence_at_same_applied() {
    let applied = log_id(2, 1, 3);
    let snaps = vec![
        SnapshotBuilder::new(1).applied(applied).sm_data("k", "v1").build(),
        SnapshotBuilder::new(2).applied(applied).sm_data("k", "v2").build(),
    ];

    let violations = check(&snaps);
    assert_variant!(violations, InvariantViolation::StateMachineDivergence { .. });
}

#[test]
fn state_machine_safety_different_applied_index_not_flagged() {
    // Different last_applied → incomparable by our rule; must not flag.
    let snaps = vec![
        SnapshotBuilder::new(1).applied(log_id(1, 1, 1)).sm_data("k", "v1").build(),
        SnapshotBuilder::new(2).applied(log_id(1, 1, 2)).sm_data("k", "v2").build(),
    ];

    let violations = check(&snaps);
    assert!(
        !violations.iter().any(|v| matches!(v, InvariantViolation::StateMachineDivergence { .. })),
        "different applied indexes must not trigger StateMachineDivergence: {violations:?}"
    );
}

#[test]
fn committed_on_quorum_violation() {
    // Leader n1 reports committed(1,1,2). Among voters {1,2,3}, only n1 has it.
    // 1 of 3 is not a quorum → violation.
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(1)
            .leader(1, 1, true)
            .state(openraft::ServerState::Leader)
            .log_entries([log_id(1, 1, 1), log_id(1, 1, 2)])
            .committed(log_id(1, 1, 2))
            .membership_voters(vec![vec![1, 2, 3]])
            .build(),
        SnapshotBuilder::new(2).build(),
        SnapshotBuilder::new(3).build(),
    ];

    let violations = check(&snaps);
    assert_variant!(violations, InvariantViolation::CommittedNotOnQuorum { .. });
}

#[test]
fn committed_on_quorum_exact_majority_ok() {
    // 2 of 3 voters have the entry → legitimate majority.
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(1)
            .leader(1, 1, true)
            .state(openraft::ServerState::Leader)
            .log_entries([log_id(1, 1, 1), log_id(1, 1, 2)])
            .committed(log_id(1, 1, 2))
            .membership_voters(vec![vec![1, 2, 3]])
            .build(),
        SnapshotBuilder::new(2).log_entries([log_id(1, 1, 1), log_id(1, 1, 2)]).build(),
        SnapshotBuilder::new(3).build(),
    ];

    let violations = check(&snaps);
    assert!(
        !violations.iter().any(|v| matches!(v, InvariantViolation::CommittedNotOnQuorum { .. })),
        "2-of-3 is a majority and must not be flagged: {violations:?}"
    );
}

#[test]
fn committed_on_quorum_counts_snapshot_applied_prefix() {
    let compacted = log_id(3, 2, 10);
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(2)
            .leader(2, 1, true)
            .state(openraft::ServerState::Leader)
            .log_entries([log_id(1, 1, 1), log_id(2, 1, 2)])
            .committed(log_id(2, 1, 2))
            .membership_voters(vec![vec![1, 2, 3]])
            .build(),
        SnapshotBuilder::new(2)
            .term(3)
            .snapshot(compacted)
            .purged(compacted)
            .committed(compacted)
            .applied(compacted)
            .build(),
        SnapshotBuilder::new(3).build(),
    ];

    let violations = check(&snaps);
    assert!(
        !violations.iter().any(|v| matches!(v, InvariantViolation::CommittedNotOnQuorum { .. })),
        "snapshot-applied voter must count toward CommittedOnQuorum: {violations:?}"
    );
}

#[test]
fn committed_on_quorum_uses_old_config_before_joint_membership_log() {
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(1)
            .leader(1, 1, true)
            .state(openraft::ServerState::Leader)
            .log_entries([log_id(1, 1, 1), log_id(1, 1, 2), log_id(1, 1, 3)])
            .committed(log_id(1, 1, 2))
            .membership_voters(vec![vec![1, 2, 3], vec![1, 2, 3, 4]])
            .membership_log_id(log_id(1, 1, 3))
            .build(),
        SnapshotBuilder::new(2).log_entries([log_id(1, 1, 1), log_id(1, 1, 2)]).build(),
        SnapshotBuilder::new(3).build(),
        SnapshotBuilder::new(4).build(),
    ];

    let violations = check(&snaps);
    assert!(
        !violations.iter().any(|v| matches!(v, InvariantViolation::CommittedNotOnQuorum { .. })),
        "pre-membership entry must be checked against the old config only: {violations:?}"
    );
}

#[test]
fn committed_on_quorum_counts_crashed_voter_durable_log() {
    let entry = log_id(1, 1, 7);
    let snaps = vec![
        SnapshotBuilder::new(1)
            .term(1)
            .leader(1, 1, true)
            .state(openraft::ServerState::Leader)
            .log_entries([entry])
            .committed(entry)
            .membership_voters(vec![vec![1, 2, 3, 4]])
            .build(),
        SnapshotBuilder::new(3).build(),
        SnapshotBuilder::new(4).log_entries([entry]).build(),
    ];
    let mut logs = durable_logs(&snaps);
    logs.insert(2, LogIdListOf::<TypeConfig>::new(None, [entry]));

    let violations = check_with_durable_logs(&snaps, &logs);
    assert!(
        !violations.iter().any(|v| matches!(v, InvariantViolation::CommittedNotOnQuorum { .. })),
        "durable log on crashed voter must count toward quorum: {violations:?}"
    );
}

#[tokio::test]
async fn committed_on_quorum_excludes_unflushed_entries() {
    let log_id = log_id(1, 1, 1);
    let leader = SnapshotBuilder::new(1).term(1);
    let leader = leader.leader(1, 1, true);
    let leader = leader.state(openraft::ServerState::Leader);
    let leader = leader.log_entries([log_id]);
    let leader = leader.committed(log_id);
    let leader = leader.membership_voters(vec![vec![1, 2, 3]]);
    let snapshots = vec![leader.build()];
    let model = StorageModel {
        async_flush: true,
        flush_delay_ms: (10, 10),
        ..StorageModel::baseline()
    };
    let mut cluster = ClusterState::new();
    let mut callbacks = Vec::new();

    for node_id in [1, 2] {
        let mut store = Arc::new(LogStore::new(model.clone(), node_id));
        let entry = Entry {
            log_id,
            payload: EntryPayload::Blank,
        };
        let (tx, rx) = TypeConfig::oneshot();
        let callback = IOFlushed::signal(tx);
        let appended = store.append([entry], callback).await;
        appended.unwrap();
        cluster.log_stores.insert(node_id, store);
        callbacks.push(rx);
    }

    let logs = cluster.get_all_durable_log_ids();
    let violations = check_with_durable_logs(&snapshots, &logs);
    match violations.as_slice() {
        [
            InvariantViolation::CommittedNotOnQuorum {
                leader: 1,
                index: 1,
                voters,
                write_size: 2,
                matching,
            },
        ] => {
            assert_eq!(voters, &[1, 2, 3]);
            let expected_matching = Vec::<NodeId>::new();
            assert_eq!(matching, &expected_matching);
        }
        _ => panic!("expected an unflushed quorum violation: {violations:?}"),
    }

    for callback in callbacks {
        let flushed = callback.await.unwrap();
        flushed.unwrap();
    }
    let logs = cluster.get_all_durable_log_ids();
    let violations = check_with_durable_logs(&snapshots, &logs);
    let no_violations = violations.is_empty();
    assert!(no_violations, "flushed quorum rejected: {violations:?}");
}

#[cfg(feature = "threshold-membership")]
#[test]
fn committed_on_quorum_counts_write_size() {
    // 5 voters with read_size 4 and write_size 2: an entry commits once 2 voters store it.
    let leader = || {
        let nodes: std::collections::BTreeMap<NodeId, crate::typ::Node> =
            (1..=5).map(|id| (id, crate::typ::Node::default())).collect();
        let voters = nodes.keys().copied().collect();
        let membership = crate::typ::Membership::new(voters, 2, nodes).unwrap();

        let (id, mut snapshot) = SnapshotBuilder::new(1)
            .term(1)
            .leader(1, 1, true)
            .state(openraft::ServerState::Leader)
            .log_entries([log_id(1, 1, 1), log_id(1, 1, 2)])
            .committed(log_id(1, 1, 2))
            .build();
        snapshot.raft.membership_config = Arc::new(StoredMembership::new(Some(log_id(1, 1, 1)), membership));
        (id, snapshot)
    };

    let stored_by_two = vec![
        leader(),
        SnapshotBuilder::new(2).log_entries([log_id(1, 1, 1), log_id(1, 1, 2)]).build(),
    ];
    let violations = check(&stored_by_two);
    assert!(
        !violations.iter().any(|v| matches!(v, InvariantViolation::CommittedNotOnQuorum { .. })),
        "2 of 5 voters reach write size 2: {violations:?}"
    );

    let stored_by_leader = vec![leader(), SnapshotBuilder::new(2).build()];
    let violations = check(&stored_by_leader);
    let mut got = Vec::new();
    for violation in &violations {
        if matches!(violation, InvariantViolation::CommittedNotOnQuorum { .. }) {
            got.push(violation.to_string());
        }
    }
    let want = "CommittedOnQuorum: leader n1 committed index 2; only [1] of voters [1, 2, 3, 4, 5] \
                have matching entry, fewer than write size 2";
    assert_eq!(vec![want.to_string()], got);
}

#[test]
fn state_ordering_applied_beyond_committed() {
    let snaps = vec![
        SnapshotBuilder::new(1)
            .log_entries([log_id(1, 1, 1), log_id(1, 1, 2), log_id(1, 1, 3)])
            .committed(log_id(1, 1, 1))
            .applied(log_id(1, 1, 3))
            .build(),
    ];

    let violations = check(&snaps);
    assert_variant!(violations, InvariantViolation::StateOrdering {
        lower: "applied",
        higher: "committed",
        ..
    });
}

#[test]
fn committed_immutable_catches_cross_tick_leader_id_change() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![SnapshotBuilder::new(1).term(1).log_entries([log_id(1, 1, 1)]).committed(log_id(1, 1, 1)).build()];
    let r1 = checker.check(&tick1);
    assert_no_violations!(r1.violations);

    // Same index 1, but now with a different leader id. This is only
    // detectable as a cross-tick violation.
    let tick2 = vec![SnapshotBuilder::new(2).term(2).log_entries([log_id(2, 2, 1)]).committed(log_id(2, 2, 1)).build()];
    let r2 = checker.check(&tick2);
    assert_variant!(r2.violations, InvariantViolation::CommittedChanged { index: 1, .. });
}

#[test]
fn committed_immutable_ignores_unwitnessed_snapshot_prefix() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![SnapshotBuilder::new(1).term(1).log_entries([log_id(1, 1, 5)]).committed(log_id(1, 1, 5)).build()];
    let r1 = checker.check(&tick1);
    assert_no_violations!(r1.violations);

    let compacted = log_id(2, 2, 10);
    let tick2 = vec![
        SnapshotBuilder::new(1)
            .term(2)
            .snapshot(compacted)
            .purged(compacted)
            .committed(compacted)
            .applied(compacted)
            .build(),
    ];

    let r2 = checker.check(&tick2);
    assert_no_violations!(r2.violations);
}

#[test]
fn monotonic_term_regression() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![SnapshotBuilder::new(1).term(5).build()];
    assert_no_violations!(checker.check(&tick1).violations);

    let tick2 = vec![SnapshotBuilder::new(1).term(3).build()];
    let r = checker.check(&tick2);
    assert_variant!(r.violations, InvariantViolation::TermRegressed {
        previous: 5,
        current: 3,
        ..
    });
}

#[test]
fn monotonic_commit_index_regression() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![
        SnapshotBuilder::new(1)
            .log_entries([log_id(1, 1, 1), log_id(1, 1, 5)])
            .committed(log_id(1, 1, 5))
            .build(),
    ];
    assert_no_violations!(checker.check(&tick1).violations);

    let tick2 = vec![
        SnapshotBuilder::new(1)
            .log_entries([log_id(1, 1, 1), log_id(1, 1, 3)])
            .committed(log_id(1, 1, 3))
            .build(),
    ];
    let r = checker.check(&tick2);
    assert_variant!(r.violations, InvariantViolation::CommitIndexRegressed {
        previous: 5,
        current: 3,
        ..
    });
}

#[tokio::test]
async fn commit_index_can_recover_from_its_durable_prefix() {
    tracing::info!("Persist node 1's log and commit through index 3");
    let mut store = {
        let model = StorageModel {
            async_flush: true,
            flush_delay_ms: (10, 10),
            ..StorageModel::baseline()
        };
        let mut store = Arc::new(LogStore::new(model, 0));
        let vote = Vote::new_committed(1, 1);
        let saved = store.save_vote(&vote).await;
        saved.unwrap();
        let entries: Vec<_> = (1..=3)
            .map(|index| Entry {
                log_id: log_id(1, 1, index),
                payload: EntryPayload::Blank,
            })
            .collect();
        let (tx, rx) = TypeConfig::oneshot();
        let callback = IOFlushed::signal(tx);
        let appended = store.append(entries, callback).await;
        appended.unwrap();
        let flushed = rx.await.unwrap();
        flushed.unwrap();
        let saved = store.save_committed(Some(log_id(1, 1, 3))).await;
        saved.unwrap();
        let saved = store.save_vote(&vote).await;
        saved.unwrap();
        store
    };

    tracing::info!("Observe node 1's unflushed commit through index 5");
    let mut checker = {
        let entries: Vec<_> = (4..=5)
            .map(|index| Entry {
                log_id: log_id(1, 1, index),
                payload: EntryPayload::Blank,
            })
            .collect();
        let appended = store.append(entries, IOFlushed::noop()).await;
        appended.unwrap();
        let saved = store.save_committed(Some(log_id(1, 1, 5))).await;
        saved.unwrap();

        let mut checker = InvariantChecker::default();
        let before = SnapshotBuilder::new(1).term(1);
        let before = before.leader(1, 1, true);
        let before = before.log_entries([log_id(1, 1, 1), log_id(1, 1, 5)]);
        let before = before.committed(log_id(1, 1, 5));
        let before = before.applied(log_id(1, 1, 3));
        let before = vec![before.build()];
        let result = checker.check(&before);
        let no_violations = result.violations.is_empty();
        assert!(no_violations, "initial snapshot rejected: {:?}", result.violations);
        checker
    };

    tracing::info!("Recover node 1 at index 3 after a power loss");
    {
        store.crash(true);
        checker.on_node_crash(1);
        let recovered = store.read_committed().await.unwrap();
        assert_eq!(recovered, Some(log_id(1, 1, 3)));
        let after = SnapshotBuilder::new(1).term(1);
        let after = after.leader(1, 1, true);
        let after = after.log_entries([log_id(1, 1, 1), log_id(1, 1, 3)]);
        let after = after.committed(recovered.unwrap());
        let after = after.applied(log_id(1, 1, 3));
        let after = vec![after.build()];
        let result = checker.check(&after);
        let no_violations = result.violations.is_empty();
        assert!(no_violations, "recovered snapshot rejected: {:?}", result.violations);
    }
}

#[test]
fn crash_resets_only_the_affected_node() {
    let mut checker = InvariantChecker::default();
    let mut before = Vec::new();
    for node_id in [1, 2] {
        let snapshot = SnapshotBuilder::new(node_id).log_entries([log_id(1, 1, 1), log_id(1, 1, 5)]);
        let snapshot = snapshot.committed(log_id(1, 1, 5));
        before.push(snapshot.build());
    }
    let result = checker.check(&before);
    let no_violations = result.violations.is_empty();
    assert!(no_violations, "initial snapshots rejected: {:?}", result.violations);

    checker.on_node_crash(1);
    let mut after = Vec::new();
    for node_id in [1, 2] {
        let snapshot = SnapshotBuilder::new(node_id).log_entries([log_id(1, 1, 1), log_id(1, 1, 3)]);
        let snapshot = snapshot.committed(log_id(1, 1, 3));
        after.push(snapshot.build());
    }
    let result = checker.check(&after);
    match result.violations.as_slice() {
        [
            InvariantViolation::CommitIndexRegressed {
                node: 2,
                previous: 5,
                current: 3,
            },
        ] => {}
        _ => panic!("expected only node 2 to regress: {:?}", result.violations),
    }
}

#[test]
fn durable_history_is_still_checked_after_crash() {
    let mut checker = InvariantChecker::default();
    let before = SnapshotBuilder::new(1).term(2);
    let before = before.leader(2, 1, true);
    let before = before.log_entries([log_id(1, 1, 1), log_id(1, 1, 5)]);
    let before = before.committed(log_id(1, 1, 5));
    let before = before.applied(log_id(1, 1, 3));
    let before = vec![before.build()];
    let result = checker.check(&before);
    let no_violations = result.violations.is_empty();
    assert!(no_violations, "initial snapshot rejected: {:?}", result.violations);

    checker.on_node_crash(1);
    let after = SnapshotBuilder::new(1).term(2);
    let after = after.leader(1, 1, true);
    let after = after.log_entries([log_id(1, 1, 1), log_id(1, 1, 3)]);
    let after = after.committed(log_id(1, 1, 3));
    let after = after.applied(log_id(1, 1, 2));
    let after = vec![after.build()];
    let result = checker.check(&after);
    match result.violations.as_slice() {
        [
            InvariantViolation::AppliedIndexRegressed {
                node: 1,
                previous: 3,
                current: 2,
            },
            InvariantViolation::VoteRegressed {
                node: 1,
                previous,
                current,
            },
        ] => {
            let previous_vote = Vote::new_committed(2, 1).to_string();
            let current_vote = Vote::new_committed(1, 1).to_string();
            assert_eq!(previous, &previous_vote);
            assert_eq!(current, &current_vote);
        }
        _ => panic!("expected durable regressions: {:?}", result.violations),
    }
}

#[test]
fn monotonic_applied_index_regression() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![SnapshotBuilder::new(1).applied(log_id(1, 1, 7)).build()];
    assert_no_violations!(checker.check(&tick1).violations);

    let tick2 = vec![SnapshotBuilder::new(1).applied(log_id(1, 1, 4)).build()];
    let r = checker.check(&tick2);
    assert_variant!(r.violations, InvariantViolation::AppliedIndexRegressed {
        previous: 7,
        current: 4,
        ..
    });
}

#[test]
fn sm_key_regression_detected() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![SnapshotBuilder::new(1).sm_key_at("k", log_id(1, 1, 5)).build()];
    assert_no_violations!(checker.check(&tick1).violations);

    // The key rolled back to an older writing log id.
    let tick2 = vec![SnapshotBuilder::new(1).sm_key_at("k", log_id(1, 1, 3)).build()];
    let r = checker.check(&tick2);
    assert_variant!(r.violations, InvariantViolation::SmKeyRegressed {
        node: 1,
        current: Some(_),
        ..
    });
}

#[test]
fn sm_key_vanished_detected() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![SnapshotBuilder::new(1).sm_key_at("k", log_id(1, 1, 5)).build()];
    assert_no_violations!(checker.check(&tick1).violations);

    // Keys are never deleted: disappearing is a rollback.
    let tick2 = vec![SnapshotBuilder::new(1).build()];
    let r = checker.check(&tick2);
    assert_variant!(r.violations, InvariantViolation::SmKeyRegressed {
        node: 1,
        current: None,
        ..
    });
}

#[test]
fn sm_key_progress_is_clean() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![SnapshotBuilder::new(1).sm_key_at("k", log_id(1, 1, 5)).build()];
    assert_no_violations!(checker.check(&tick1).violations);

    // Same position, advanced position, new keys, and a new node: all fine.
    let tick2 = vec![
        SnapshotBuilder::new(1).sm_key_at("k", log_id(2, 2, 8)).sm_key_at("k2", log_id(2, 2, 9)).build(),
        SnapshotBuilder::new(2).sm_key_at("k", log_id(1, 1, 5)).build(),
    ];
    assert_no_violations!(checker.check(&tick2).violations);
}

#[test]
fn sm_key_checked_across_node_downtime() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![SnapshotBuilder::new(1).sm_key_at("k", log_id(1, 1, 5)).build()];
    assert_no_violations!(checker.check(&tick1).violations);

    // Node 1 is down: it drops out of the snapshot list entirely.
    assert_no_violations!(checker.check(&[]).violations);

    // Storage is durable, so it must come back at-or-ahead of where it was.
    let tick3 = vec![SnapshotBuilder::new(1).sm_key_at("k", log_id(1, 1, 3)).build()];
    let r = checker.check(&tick3);
    assert_variant!(r.violations, InvariantViolation::SmKeyRegressed { node: 1, .. });
}

#[test]
fn monotonic_vote_regression() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![SnapshotBuilder::new(1).term(5).leader(5, 2, true).build()];
    assert_no_violations!(checker.check(&tick1).violations);

    // Regress the vote: drop the committed flag at the same (term, leader_id).
    let tick2 = vec![SnapshotBuilder::new(1).term(5).leader(5, 2, false).build()];
    let r = checker.check(&tick2);
    assert_variant!(r.violations, InvariantViolation::VoteRegressed { .. });
}
