//! Unit tests for the invariant checkers.
//!
//! Each test constructs a handful of [`FullNodeSnapshot`]s by hand, feeds them
//! to the relevant checker, and asserts which violation variants (if any)
//! pop out. Using the public [`InvariantChecker::check`] entry point keeps the
//! tests close to the real call site, while the helpers keep the fixtures
//! readable.

use std::sync::Arc;

use openraft::alias::LogIdListOf;
use openraft::metrics::RaftMetrics;
use openraft::vote::RaftLeaderId;

use super::CLeaderId;
use super::InvariantChecker;
use super::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::store::StateMachineData;
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
    membership: Vec<Vec<NodeId>>,
    sm_last_applied: Option<LogId>,
    sm_data: std::collections::HashMap<String, String>,
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
            membership: Vec::new(),
            sm_last_applied: None,
            sm_data: std::collections::HashMap::new(),
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

    fn sm_data(mut self, k: &str, v: &str) -> Self {
        self.sm_data.insert(k.into(), v.into());
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
        raft.committed = self.committed;
        raft.last_applied = self.last_applied;
        raft.snapshot = self.snapshot;
        raft.purged = self.purged;
        raft.log_id_list = LogIdListOf::<TypeConfig>::new(self.purged, self.log_entries);

        if !self.membership.is_empty() {
            let sets: Vec<std::collections::BTreeSet<NodeId>> =
                self.membership.into_iter().map(|v| v.into_iter().collect()).collect();
            let mem = openraft::Membership::new_with_defaults(sets, std::iter::empty::<NodeId>());
            raft.membership_config = Arc::new(StoredMembership::new(None, mem));
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
fn monotonic_vote_regression() {
    let mut checker = InvariantChecker::default();

    let tick1 = vec![SnapshotBuilder::new(1).term(5).leader(5, 2, true).build()];
    assert_no_violations!(checker.check(&tick1).violations);

    // Regress the vote: drop the committed flag at the same (term, leader_id).
    let tick2 = vec![SnapshotBuilder::new(1).term(5).leader(5, 2, false).build()];
    let r = checker.check(&tick2);
    assert_variant!(r.violations, InvariantViolation::VoteRegressed { .. });
}
