//! State Machine Safety (Paper §3.6.3):
//!
//! > If a server has applied a log entry at a given index to its state machine,
//! > no other server will ever apply a different log entry for the same index.
//!
//! Since our state machine only exposes the full current KV map (not per-index
//! deltas), the strongest pointwise check we can do is:
//!
//! > Two nodes whose `last_applied` **log id** is identical must have identical
//! > state machine data.
//!
//! Same index but different `CLeaderId` at `last_applied` is *also* a
//! violation, but it is an implied consequence of Log Matching, so the Log
//! Matching check is the one that reports it.

use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::NodeId;

/// Compare two nodes' state-machine data when they stand at exactly the same log id.
pub fn check(a: &(NodeId, FullNodeSnapshot), b: &(NodeId, FullNodeSnapshot), violations: &mut Vec<InvariantViolation>) {
    let (id_a, s_a) = a;
    let (id_b, s_b) = b;

    let (Some(applied_a), Some(applied_b)) = (s_a.sm.last_applied, s_b.sm.last_applied) else {
        return;
    };

    // Comparable only when both have applied to *exactly* the same log id.
    // (Same-index/different-leader is caught by log_matching.)
    if applied_a != applied_b {
        return;
    }

    if s_a.sm.data == s_b.sm.data {
        return;
    }

    violations.push(InvariantViolation::StateMachineDivergence {
        index: applied_a.index(),
        nodes: vec![*id_a, *id_b],
    });
}
