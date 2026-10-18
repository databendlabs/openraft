//! Log Id Uniqueness (derived from Log Matching, Paper §3.5).
//!
//! A log id names one entry: a leader creates at most one entry per index, so
//! no two nodes ever hold different entries under the same log id. Log
//! Matching compares log ids only and cannot see two payloads behind one id;
//! this check compares the payloads.
//!
//! openraft [#2167] breaks it: a leader that crashes before flushing entries
//! it already replicated, then restores its leadership, reissues their log
//! ids for new entries.
//!
//! Two forms:
//!
//! - [`LogIdWitness`] checks each append when it happens, against the entries other nodes still
//!   hold under that log id. An entry no node holds any more, e.g. one lost with the power before
//!   it reached anyone, may be reissued harmlessly.
//! - [`check_final`] compares every pair of logs at the end of a run.
//!
//! [#2167]: https://github.com/databendlabs/openraft/issues/2167

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::violation::InvariantViolation;
use crate::store::AppendedEntry;
use crate::typ::LogId;
use crate::typ::NodeId;

/// A payload appended under some log id, with the nodes that appended it.
type Version = (Vec<u8>, BTreeSet<NodeId>);

/// Every payload ever appended under each log id.
#[derive(Default)]
pub struct LogIdWitness {
    seen: BTreeMap<LogId, Vec<Version>>,
}

impl LogIdWitness {
    /// Check `appended` against what other nodes hold now, then record it.
    ///
    /// `payload_at(node, log_id)` returns what `node`'s log holds under
    /// `log_id`, if anything.
    pub fn check_and_record(
        &mut self,
        appended: Vec<AppendedEntry>,
        payload_at: impl Fn(NodeId, &LogId) -> Option<Vec<u8>>,
        violations: &mut Vec<InvariantViolation>,
    ) {
        for entry in appended {
            let versions = self.seen.entry(entry.log_id).or_default();

            let holder =
                versions.iter().filter(|(payload, _)| *payload != entry.payload).find_map(|(payload, nodes)| {
                    nodes.iter().copied().find(|node| payload_at(*node, &entry.log_id).as_ref() == Some(payload))
                });
            if let Some(holder) = holder {
                violations.push(InvariantViolation::LogIdReused {
                    log_id: entry.log_id,
                    holder,
                    appender: entry.node,
                });
            }

            match versions.iter_mut().find(|(payload, _)| *payload == entry.payload) {
                Some((_, nodes)) => {
                    nodes.insert(entry.node);
                }
                None => versions.push((entry.payload, BTreeSet::from([entry.node]))),
            }
        }
    }
}

/// Compare every node's log with every other's: one log id, one payload.
pub fn check_final(logs: &BTreeMap<NodeId, Vec<(LogId, Vec<u8>)>>, violations: &mut Vec<InvariantViolation>) {
    let mut first_holder: BTreeMap<LogId, (NodeId, &Vec<u8>)> = BTreeMap::new();
    let mut reported: BTreeSet<LogId> = BTreeSet::new();

    for (node, entries) in logs {
        for (log_id, payload) in entries {
            match first_holder.get(log_id) {
                None => {
                    first_holder.insert(*log_id, (*node, payload));
                }
                Some((holder, held)) => {
                    if *held != payload && reported.insert(*log_id) {
                        violations.push(InvariantViolation::LogIdReused {
                            log_id: *log_id,
                            holder: *holder,
                            appender: *node,
                        });
                    }
                }
            }
        }
    }
}
