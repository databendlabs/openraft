//! State Ordering (implementation sanity).
//!
//! On any node at any time:
//!
//! ```text
//! purged.index ≤ snapshot.index ≤ applied.index ≤ committed.index ≤ last_log_index
//! ```
//!
//! This is not a Raft safety property per se — the paper and TLA+ specs do not
//! name it — but every correct implementation must preserve it. A violation
//! indicates internal corruption (e.g., an out-of-order state update).

use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::NodeId;

/// Check `purged ≤ snapshot ≤ applied ≤ committed ≤ last_log` on one node.
pub fn check(node: NodeId, s: &FullNodeSnapshot, violations: &mut Vec<InvariantViolation>) {
    let purged = s.raft.purged.as_ref().map(|id| id.index());
    let snapshot = s.raft.snapshot.as_ref().map(|id| id.index());
    let applied = s.raft.last_applied.as_ref().map(|id| id.index());
    let committed = s.raft.committed.as_ref().map(|id| id.index());
    let last_log = s.raft.last_log_index;

    let pairs = [
        ("purged", purged, "snapshot", snapshot),
        ("snapshot", snapshot, "applied", applied),
        ("applied", applied, "committed", committed),
        ("committed", committed, "last_log", last_log),
    ];

    for (lower_name, lower, higher_name, higher) in pairs {
        if let (Some(x), Some(y)) = (lower, higher)
            && x > y
        {
            violations.push(InvariantViolation::StateOrdering {
                node,
                lower: lower_name,
                lower_index: x,
                higher: higher_name,
                higher_index: y,
            });
        }
    }
}
