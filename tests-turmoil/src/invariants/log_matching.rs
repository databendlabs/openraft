//! Log Matching (Paper §3.5, Vanlightly `NoLogDivergence`).
//!
//! Formal statement (paraphrased):
//!
//! > If two logs contain an entry with the same index and term, then the logs
//! > are identical in all preceding entries.
//!
//! To avoid false positives from uncommitted entries during leader transitions
//! (which may legitimately be truncated), this check only compares indices
//! that **both** nodes believe are committed.
//!
//! # Purged-boundary handling
//!
//! The comparison range includes `max(purged_a, purged_b)` as its lower bound.
//! `LogIdList::get(index)` returns the purged log id when `index == purged.index`,
//! so a purged-but-still-observable entry at the exact boundary can be checked.
//! Starting one past the purged index (the previous behavior) silently skipped
//! the boundary and could let a divergence slip past right at the snapshot edge.

use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::LogId;
use crate::typ::NodeId;

/// Compare committed entries on any two nodes index-by-index.
pub fn check(a: &(NodeId, FullNodeSnapshot), b: &(NodeId, FullNodeSnapshot), violations: &mut Vec<InvariantViolation>) {
    let (id_a, s_a) = a;
    let (id_b, s_b) = b;

    let last_a = s_a.raft.committed.map(|id: LogId| id.index()).unwrap_or(0);
    let last_b = s_b.raft.committed.map(|id: LogId| id.index()).unwrap_or(0);

    // Include the purged entry itself — `LogIdList::get(purged.index)` returns
    // the purged log id, so we can still compare at the boundary.
    let first_a = s_a.raft.log_id_list.purged().map(|id: &LogId| id.index()).unwrap_or(0);
    let first_b = s_b.raft.log_id_list.purged().map(|id: &LogId| id.index()).unwrap_or(0);

    let start = first_a.max(first_b);
    let end = last_a.min(last_b);

    for idx in start..=end {
        let lid_a = s_a.raft.log_id_list.get(idx).map(|id| *id.committed_leader_id());
        let lid_b = s_b.raft.log_id_list.get(idx).map(|id| *id.committed_leader_id());

        if let (Some(la), Some(lb)) = (lid_a, lid_b)
            && la != lb
        {
            violations.push(InvariantViolation::LogDivergence {
                index: idx,
                node_a: *id_a,
                lid_a: la,
                node_b: *id_b,
                lid_b: lb,
            });
        }
    }
}
