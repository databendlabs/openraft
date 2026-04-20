//! Monotonic per-node properties (Vanlightly `MonotonicTerm`,
//! `MonotonicCommitIndex`, and friends).
//!
//! Every one of these is a *temporal* property: it compares a node at tick
//! `t` with the same node at tick `t-1`. The checker keeps one [`LastSeen`]
//! record per node and updates it on every invocation.
//!
//! # Properties covered
//!
//! | Property                 | Source          | What it says               |
//! |--------------------------|-----------------|----------------------------|
//! | `MonotonicTerm`          | Vanlightly      | `current_term` never decreases |
//! | `MonotonicCommitIndex`   | Vanlightly      | `committed.index` never decreases |
//! | `MonotonicAppliedIndex`  | derived         | `applied.index` never decreases |
//! | `MonotonicVote`          | Vanlightly      | persisted vote is non-decreasing in `(term, leader_id, committed)` order |
//!
//! Applied-monotonic is not named in Vanlightly's spec but follows from two
//! invariants the spec does have: `applied ≤ committed` and `MonotonicCommit`.
//! Checking it explicitly catches bugs where an apply step accidentally
//! re-runs on a rolled-back state.

use std::collections::BTreeMap;

use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::NodeId;
use crate::typ::Vote;

/// Last-seen-per-node snapshot of the four monotonic quantities.
#[derive(Debug, Clone)]
pub struct LastSeen {
    pub term: u64,
    pub committed_index: u64,
    pub applied_index: u64,
    pub vote: Vote,
}

/// Cross-tick monotonic-property checker. One record per node.
#[derive(Default)]
pub struct MonotonicHistory {
    seen: BTreeMap<NodeId, LastSeen>,
}

impl MonotonicHistory {
    pub fn check_and_record(
        &mut self,
        snapshots: &[(NodeId, FullNodeSnapshot)],
        violations: &mut Vec<InvariantViolation>,
    ) {
        for (node_id, s) in snapshots {
            let current = observe(s);

            if let Some(prev) = self.seen.get(node_id) {
                diff_against_previous(*node_id, prev, &current, violations);
            }

            self.seen.insert(*node_id, current);
        }
    }
}

/// Extract the monotonic quantities from one full snapshot.
fn observe(s: &FullNodeSnapshot) -> LastSeen {
    LastSeen {
        term: s.raft.current_term,
        committed_index: s.raft.committed.as_ref().map(|id| id.index()).unwrap_or(0),
        applied_index: s.raft.last_applied.as_ref().map(|id| id.index()).unwrap_or(0),
        vote: s.raft.vote,
    }
}

/// Compare `current` against `prev`, pushing a violation for each regression.
fn diff_against_previous(node: NodeId, prev: &LastSeen, current: &LastSeen, violations: &mut Vec<InvariantViolation>) {
    if current.term < prev.term {
        violations.push(InvariantViolation::TermRegressed {
            node,
            previous: prev.term,
            current: current.term,
        });
    }

    if current.committed_index < prev.committed_index {
        violations.push(InvariantViolation::CommitIndexRegressed {
            node,
            previous: prev.committed_index,
            current: current.committed_index,
        });
    }

    if current.applied_index < prev.applied_index {
        violations.push(InvariantViolation::AppliedIndexRegressed {
            node,
            previous: prev.applied_index,
            current: current.applied_index,
        });
    }

    // Raft persists `(term, leader_id, committed)` and guarantees it is totally
    // ordered and non-regressing. `Vote: Ord` captures that ordering; a strict
    // `current < prev` is a regression.
    if current.vote < prev.vote {
        violations.push(InvariantViolation::VoteRegressed {
            node,
            previous: format!("{}", prev.vote),
            current: format!("{}", current.vote),
        });
    }
}
