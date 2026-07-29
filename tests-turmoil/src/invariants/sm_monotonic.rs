//! Per-node, per-key state-machine monotonicity (derived).
//!
//! A node applies only committed entries, in log order, and an installed
//! snapshot must be at or ahead of the node's applied state — so on any
//! single node, the log id that last wrote a key can only move forward, and
//! a key can never disappear. Checking this per tick makes *every* node's
//! state machine content observable to the checker: a follower's key-level
//! rollback (apply-before-commit, bad snapshot handling) no longer hides
//! until the node happens to share an applied log id with another node
//! (State Machine Safety) or until the final durability scan.
//!
//! No reset-on-restart is needed: storage in this harness is durable across
//! crashes (a bounced node keeps its log store and state machine), and a
//! crashed node drops out of the snapshot list while down, so it resumes
//! at-or-ahead of its last observed state.

use std::collections::BTreeMap;

use super::violation::InvariantViolation;
use crate::cluster::FullNodeSnapshot;
use crate::typ::LogId;
use crate::typ::NodeId;

/// Cross-tick per-key state-machine monotonicity checker. One key map per
/// node, updated on every invocation.
#[derive(Default)]
pub struct SmMonotonicHistory {
    /// Per node: last seen (key -> log id of the entry that wrote it).
    seen: BTreeMap<NodeId, BTreeMap<String, LogId>>,
}

impl SmMonotonicHistory {
    pub fn check_and_record(
        &mut self,
        snapshots: &[(NodeId, FullNodeSnapshot)],
        violations: &mut Vec<InvariantViolation>,
    ) {
        for (node_id, s) in snapshots {
            let current: BTreeMap<String, LogId> = s.sm.data.iter().map(|(k, m)| (k.clone(), m.log_id)).collect();

            if let Some(prev) = self.seen.get(node_id) {
                for (key, prev_log_id) in prev {
                    let now = current.get(key);
                    let regressed = match now {
                        Some(log_id) => log_id < prev_log_id,
                        None => true,
                    };
                    if regressed {
                        violations.push(InvariantViolation::SmKeyRegressed {
                            node: *node_id,
                            key: key.clone(),
                            previous: *prev_log_id,
                            current: now.copied(),
                        });
                    }
                }
            }

            self.seen.insert(*node_id, current);
        }
    }
}
