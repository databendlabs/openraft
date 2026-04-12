//! Raft invariant checking.
//!
//! Each check function corresponds to a Raft safety property:
//! - Election Safety: at most one leader per term
//! - Leader Completeness: committed entries must be in every later leader's log
//! - Log Consistency: committed entries at the same index must agree
//! - State Machine Safety: same applied prefix implies identical state

use std::collections::HashMap;

use openraft::vote::RaftLeaderId;

use crate::cluster::FullNodeSnapshot;
use crate::typ::*;

/// Result of invariant checking.
#[derive(Debug)]
pub struct InvariantCheckResult {
    pub passed: bool,
    pub violations: Vec<InvariantViolation>,
}

impl InvariantCheckResult {
    pub fn with_violations(violations: Vec<InvariantViolation>) -> Self {
        Self {
            passed: violations.is_empty(),
            violations,
        }
    }
}

/// Types of invariant violations.
#[derive(Debug, Clone)]
pub enum InvariantViolation {
    /// Multiple leaders elected in the same term.
    MultipleLeadersInTerm { term: u64, leaders: Vec<NodeId> },

    /// Log entries at the same index have different leader IDs.
    LogMismatch {
        index: u64,
        node_a: NodeId,
        node_b: NodeId,
        term_a: u64,
        term_b: u64,
    },

    /// Different nodes applied the same log prefix but have different state.
    StateMachineDivergence { index: u64, nodes: Vec<NodeId> },

    /// A committed entry is missing from a later leader's log.
    LeaderMissingCommitted {
        term: u64,
        leader: NodeId,
        missing_index: u64,
    },
}

impl std::fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MultipleLeadersInTerm { term, leaders } => {
                write!(f, "Multiple leaders {leaders:?} in term {term}")
            }
            Self::LogMismatch {
                index,
                node_a,
                node_b,
                term_a,
                term_b,
            } => {
                write!(
                    f,
                    "Log mismatch at index {index}: node {node_a} has term {term_a}, node {node_b} has term {term_b}"
                )
            }
            Self::StateMachineDivergence { index, nodes } => {
                write!(f, "State machine divergence at index {index} across nodes {nodes:?}")
            }
            Self::LeaderMissingCommitted {
                term,
                leader,
                missing_index,
            } => {
                write!(
                    f,
                    "Leader {leader} in term {term} missing committed entry at index {missing_index}"
                )
            }
        }
    }
}

/// Check all Raft invariants against the current cluster state.
pub fn check_state_invariants(snapshots: &[(NodeId, FullNodeSnapshot)]) -> InvariantCheckResult {
    let mut violations = Vec::new();

    check_election_safety(snapshots, &mut violations);
    check_leader_completeness(snapshots, &mut violations);

    for i in 0..snapshots.len() {
        for j in (i + 1)..snapshots.len() {
            check_log_consistency(&snapshots[i], &snapshots[j], &mut violations);
            check_sm_safety(&snapshots[i], &snapshots[j], &mut violations);
        }
    }

    InvariantCheckResult::with_violations(violations)
}

/// Election Safety: at most one leader per term.
fn check_election_safety(snapshots: &[(NodeId, FullNodeSnapshot)], violations: &mut Vec<InvariantViolation>) {
    let mut leaders_by_term: HashMap<u64, Vec<NodeId>> = HashMap::new();
    for (node_id, s) in snapshots {
        if s.raft.state == openraft::ServerState::Leader {
            leaders_by_term.entry(s.raft.vote.leader_id().term).or_default().push(*node_id);
        }
    }

    for (term, leaders) in &leaders_by_term {
        if leaders.len() > 1 {
            violations.push(InvariantViolation::MultipleLeadersInTerm {
                term: *term,
                leaders: leaders.clone(),
            });
        }
    }
}

/// Leader Completeness: a committed entry must be present in every later leader's log.
fn check_leader_completeness(snapshots: &[(NodeId, FullNodeSnapshot)], violations: &mut Vec<InvariantViolation>) {
    // Find leaders: nodes whose committed vote points to themselves.
    // is_committed() distinguishes actual leaders from candidates.
    let mut leaders: HashMap<u64, NodeId> = HashMap::new();
    for (node_id, s) in snapshots {
        if !s.raft.vote.is_committed() {
            continue;
        }
        let lid = s.raft.vote.leader_id();
        if lid.node_id == *node_id {
            leaders.entry(lid.term).or_insert(*node_id);
        }
    }

    // Collect all committed log entries from all nodes.
    let mut committed_logs: HashMap<u64, LogId> = HashMap::new();
    for (_, s) in snapshots {
        let Some(committed) = s.raft.committed else { continue };
        for idx in 0..=committed.index() {
            if let Some(log_id) = s.raft.log_id_list.get(idx) {
                let existing = committed_logs.get(&idx);
                if existing.is_none() || existing.unwrap().committed_leader_id() > log_id.committed_leader_id() {
                    committed_logs.insert(idx, log_id);
                }
            }
        }
    }

    // Verify each leader has all previously-committed entries.
    for (leader_term, leader_nid) in &leaders {
        let s = &snapshots.iter().find(|(id, _)| id == leader_nid).unwrap().1;
        let leader_committed_lid = s.raft.vote.leader_id().to_committed();
        let purged_up_to = s.raft.log_id_list.purged().map(|id| id.index()).unwrap_or(0);

        for (idx, committed_id) in &committed_logs {
            // Only check entries committed in earlier terms.
            if committed_id.committed_leader_id() >= &leader_committed_lid {
                continue;
            }

            match s.raft.log_id_list.get(*idx) {
                Some(lid) if lid.committed_leader_id() != committed_id.committed_leader_id() => {
                    violations.push(InvariantViolation::LeaderMissingCommitted {
                        term: *leader_term,
                        leader: *leader_nid,
                        missing_index: *idx,
                    });
                }
                None if *idx > purged_up_to => {
                    violations.push(InvariantViolation::LeaderMissingCommitted {
                        term: *leader_term,
                        leader: *leader_nid,
                        missing_index: *idx,
                    });
                }
                _ => {}
            }
        }
    }
}

/// Log Consistency: committed entries at the same index must have the same leader ID.
fn check_log_consistency(
    a: &(NodeId, FullNodeSnapshot),
    b: &(NodeId, FullNodeSnapshot),
    violations: &mut Vec<InvariantViolation>,
) {
    let (id_a, s_a) = a;
    let (id_b, s_b) = b;

    let last_a = s_a.raft.committed.map(|id: LogId| id.index()).unwrap_or(0);
    let last_b = s_b.raft.committed.map(|id: LogId| id.index()).unwrap_or(0);
    let first_a = s_a.raft.log_id_list.purged().map(|id: &LogId| id.index()).unwrap_or(0);
    let first_b = s_b.raft.log_id_list.purged().map(|id: &LogId| id.index()).unwrap_or(0);

    let start = std::cmp::max(first_a, first_b);
    let end = std::cmp::min(last_a, last_b);

    for idx in start..=end {
        let lid_a = s_a.raft.log_id_list.get(idx).map(|id: LogId| *id.committed_leader_id());
        let lid_b = s_b.raft.log_id_list.get(idx).map(|id: LogId| *id.committed_leader_id());

        if let (Some(la), Some(lb)) = (lid_a, lid_b) {
            if la != lb {
                violations.push(InvariantViolation::LogMismatch {
                    index: idx,
                    node_a: *id_a,
                    node_b: *id_b,
                    term_a: la.term,
                    term_b: lb.term,
                });
            }
        }
    }
}

/// State Machine Safety: nodes that applied the same log prefix must have identical state.
fn check_sm_safety(
    a: &(NodeId, FullNodeSnapshot),
    b: &(NodeId, FullNodeSnapshot),
    violations: &mut Vec<InvariantViolation>,
) {
    let (id_a, s_a) = a;
    let (id_b, s_b) = b;

    let (Some(applied_a), Some(applied_b)) = (s_a.sm.last_applied, s_b.sm.last_applied) else {
        return;
    };

    if applied_a != applied_b || s_a.sm.data == s_b.sm.data {
        return;
    }

    violations.push(InvariantViolation::StateMachineDivergence {
        index: applied_a.index(),
        nodes: vec![*id_a, *id_b],
    });
}
