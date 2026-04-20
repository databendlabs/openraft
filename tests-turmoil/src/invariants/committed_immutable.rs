//! Committed Immutable (history-based derivation of Log Matching).
//!
//! Once any node reports index `i` committed with leader id `L`, no node may
//! ever report index `i` with a different leader id — regardless of whether
//! the two reports come from the same tick. Logically this follows from Log
//! Matching plus no-rewrite-of-committed-entries, but an explicit history
//! check catches two classes of bugs that single-tick Log Matching cannot:
//!
//! 1. An entry gets silently replaced at some index between ticks (e.g., a buggy truncation of
//!    committed storage).
//! 2. A committed entry is re-observed after a snapshot install with a different leader id.
//!
//! # Purged-boundary handling
//!
//! `LogIdList::get(purged.index)` returns the purged log id; the iteration
//! starts at `purged.index` (not `purged.index + 1`) so we keep witnessing
//! — and re-verifying — the very last purged entry on any node that still
//! exposes it.

use super::CLeaderId;
use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::NodeId;

/// Per-index history of committed log ids observed across ticks.
#[derive(Default)]
pub struct Witness {
    /// index -> (first observed CLeaderId, node that first reported it)
    seen: std::collections::HashMap<u64, (CLeaderId, NodeId)>,
}

impl Witness {
    /// Update the witness with the current snapshots, and report any
    /// cross-tick `CLeaderId` change at an already-committed index.
    pub fn check_and_record(
        &mut self,
        snapshots: &[(NodeId, FullNodeSnapshot)],
        violations: &mut Vec<InvariantViolation>,
    ) {
        for (node_id, s) in snapshots {
            let Some(committed) = s.raft.committed else { continue };

            // Include the purged entry (boundary) — see module docs.
            let first = s.raft.log_id_list.purged().map(|id| id.index()).unwrap_or(0);
            for idx in first..=committed.index() {
                let Some(lid) = s.raft.log_id_list.get(idx) else {
                    continue;
                };
                let clid = *lid.committed_leader_id();

                match self.seen.get(&idx) {
                    None => {
                        self.seen.insert(idx, (clid, *node_id));
                    }
                    Some((orig, _)) if *orig == clid => {
                        // Still agrees — no violation.
                    }
                    Some((orig, orig_observer)) => {
                        violations.push(InvariantViolation::CommittedChanged {
                            index: idx,
                            original: *orig,
                            original_observer: *orig_observer,
                            new: clid,
                            new_observer: *node_id,
                        });
                    }
                }
            }
        }
    }
}
