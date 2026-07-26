//! Client-observation oracle.
//!
//! The server-side invariants in [`crate::invariants`] check Raft's internal
//! safety properties. This module checks what a *client* is promised:
//! linearizability of acknowledged writes and `ReadIndex` reads, per key.
//!
//! The state machine stores, for every key, the [`LogId`] of the entry that
//! last wrote it ([`crate::store::ValueMeta`]). Log ids are totally ordered,
//! so per-key observations can be compared directly:
//!
//! - **Phantom value**: an observed value must correspond to a tracked write attempt (same serial,
//!   key and value). Anything else is corruption.
//! - **Monotonic reads**: sequential linearizable reads of one key must observe non-decreasing log
//!   ids. Keys are never deleted, so a key can never be observed absent after being observed
//!   present.
//! - **Read-your-writes**: a read that *starts* after a write to the key was acked must observe
//!   that write or a later one.
//! - **Lost acked write** (final): after the cluster heals, every key's value must sit at or above
//!   the highest acked log id for that key.
//!
//! A failed or timed-out write is recorded as [`WriteOutcome::Unknown`], never
//! as "guaranteed absent": openraft may fail a `client_write` call (e.g. on
//! leadership loss) after the entry was appended, and the entry can still
//! commit later. No check in this module assumes an unknown write is absent.

use std::collections::BTreeMap;

use crate::store::StateMachineData;
use crate::store::ValueMeta;
use crate::typ::LogId;
use crate::typ::NodeId;

/// Outcome of one tracked client write.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteOutcome {
    /// Submitted; no response observed yet. May commit at any time.
    Pending,
    /// Acked as committed and applied at this log id.
    Acked(LogId),
    /// The call failed or timed out. The write may still commit later.
    Unknown,
}

/// One tracked client write.
#[derive(Debug, Clone)]
pub struct WriteAttempt {
    pub key: String,
    pub value: String,
    pub outcome: WriteOutcome,
}

/// A violation of client-observable consistency.
#[derive(Debug, Clone)]
pub enum OracleViolation {
    /// A value was observed that no tracked write produced.
    PhantomValue { key: String, serial: u64, value: String },
    /// Sequential linearizable reads of one key observed log ids going
    /// backwards (`observed: None` = key present before, absent now).
    NonMonotonicRead {
        key: String,
        prev: LogId,
        observed: Option<LogId>,
    },
    /// A read that started after an ack observed an older value.
    StaleRead {
        key: String,
        acked: LogId,
        observed: Option<LogId>,
    },
    /// After healing, a key ended below the highest acked log id.
    LostAckedWrite {
        node: NodeId,
        key: String,
        acked: LogId,
        found: Option<LogId>,
    },
}

impl std::fmt::Display for OracleViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PhantomValue { key, serial, value } => {
                write!(
                    f,
                    "PhantomValue: key={key} holds serial={serial} value={value} never written"
                )
            }
            Self::NonMonotonicRead { key, prev, observed } => {
                write!(f, "NonMonotonicRead: key={key} observed {prev} then {observed:?}")
            }
            Self::StaleRead { key, acked, observed } => {
                write!(
                    f,
                    "StaleRead: key={key} acked at {acked}, later read observed {observed:?}"
                )
            }
            Self::LostAckedWrite {
                node,
                key,
                acked,
                found,
            } => {
                write!(
                    f,
                    "LostAckedWrite: n{node} key={key} acked at {acked}, final state has {found:?}"
                )
            }
        }
    }
}

/// Aggregate workload counters, reported in the final summary.
#[derive(Debug, Default, Clone)]
pub struct WorkloadStats {
    pub writes_attempted: u64,
    pub writes_acked: u64,
    pub writes_failed: u64,
    pub reads_ok: u64,
    pub reads_failed: u64,
}

impl std::fmt::Display for WorkloadStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "writes attempted/acked/failed={}/{}/{}, reads ok/failed={}/{}",
            self.writes_attempted, self.writes_acked, self.writes_failed, self.reads_ok, self.reads_failed
        )
    }
}

/// History of every tracked client operation, plus the checks over it.
///
/// Shared (behind a mutex) between the fuzz main loop, the write driver and
/// the read client. Violations accumulate here and are drained by the main
/// loop after every tick.
#[derive(Default)]
pub struct ClientHistory {
    /// serial -> attempt. Serials are unique across one simulation.
    attempts: BTreeMap<u64, WriteAttempt>,
    /// Per key: highest acked (log_id, serial).
    max_acked: BTreeMap<String, (LogId, u64)>,
    /// Per key: log id observed by the latest completed read.
    last_read: BTreeMap<String, LogId>,
    violations: Vec<OracleViolation>,
    pub stats: WorkloadStats,
}

impl ClientHistory {
    /// Record a write the moment it is enqueued, before it can reach any node.
    pub fn record_write_attempt(&mut self, serial: u64, key: String, value: String) {
        self.stats.writes_attempted += 1;
        self.attempts.insert(serial, WriteAttempt {
            key,
            value,
            outcome: WriteOutcome::Pending,
        });
    }

    /// Record a successful `client_write` response.
    pub fn record_write_acked(&mut self, serial: u64, log_id: LogId) {
        self.stats.writes_acked += 1;
        let Some(attempt) = self.attempts.get_mut(&serial) else {
            return;
        };
        attempt.outcome = WriteOutcome::Acked(log_id);

        let key = attempt.key.clone();
        let is_newer = self.max_acked.get(&key).is_none_or(|(acked, _)| log_id > *acked);
        if is_newer {
            self.max_acked.insert(key, (log_id, serial));
        }
    }

    /// Record a failed or timed-out `client_write` call.
    pub fn record_write_failed(&mut self, serial: u64) {
        self.stats.writes_failed += 1;
        if let Some(attempt) = self.attempts.get_mut(&serial) {
            attempt.outcome = WriteOutcome::Unknown;
        }
    }

    /// The read-your-writes floor for `key`: the highest log id acked so far.
    ///
    /// Must be captured *before* the read starts; any ack recorded by then
    /// strictly precedes the read in real-time order.
    pub fn acked_floor(&self, key: &str) -> Option<LogId> {
        self.max_acked.get(key).map(|(log_id, _)| *log_id)
    }

    pub fn record_read_failed(&mut self) {
        self.stats.reads_failed += 1;
    }

    /// Record and check one completed linearizable read.
    ///
    /// `floor` is the value of [`Self::acked_floor`] captured before the read
    /// started.
    pub fn record_read(&mut self, key: &str, observed: Option<&ValueMeta>, floor: Option<LogId>) {
        self.stats.reads_ok += 1;

        if let Some(meta) = observed {
            self.check_value_is_attempted(key, meta);
        }

        // Monotonic reads: this client reads sequentially, so per key the
        // observed log id must never move backwards.
        if let Some(prev) = self.last_read.get(key) {
            let regressed = match observed {
                Some(meta) => meta.log_id < *prev,
                None => true,
            };
            if regressed {
                self.violations.push(OracleViolation::NonMonotonicRead {
                    key: key.to_string(),
                    prev: *prev,
                    observed: observed.map(|m| m.log_id),
                });
            }
        }

        // Read-your-writes: acks recorded before the read started must be visible.
        if let Some(acked) = floor {
            let stale = match observed {
                Some(meta) => meta.log_id < acked,
                None => true,
            };
            if stale {
                self.violations.push(OracleViolation::StaleRead {
                    key: key.to_string(),
                    acked,
                    observed: observed.map(|m| m.log_id),
                });
            }
        }

        if let Some(meta) = observed {
            self.last_read.insert(key.to_string(), meta.log_id);
        }
    }

    /// Check every acked write against the final, healed state machines.
    ///
    /// Call only after convergence: each member must hold, for every key with
    /// an acked write, a value at or above the acked log id.
    pub fn check_final_durability(&mut self, members: &[(NodeId, StateMachineData)]) {
        for (node, sm) in members {
            for (key, (acked, _serial)) in &self.max_acked {
                let found = sm.data.get(key);
                let lost = match found {
                    Some(meta) => meta.log_id < *acked,
                    None => true,
                };
                if lost {
                    self.violations.push(OracleViolation::LostAckedWrite {
                        node: *node,
                        key: key.clone(),
                        acked: *acked,
                        found: found.map(|m| m.log_id),
                    });
                }
            }
            for (key, meta) in &sm.data {
                self.check_value_is_attempted(key, meta);
            }
        }
    }

    /// Take all violations collected since the last drain.
    pub fn drain_violations(&mut self) -> Vec<OracleViolation> {
        std::mem::take(&mut self.violations)
    }

    /// An observed value must exactly match a tracked write attempt.
    fn check_value_is_attempted(&mut self, key: &str, meta: &ValueMeta) {
        let matches = self
            .attempts
            .get(&meta.serial)
            .is_some_and(|attempt| attempt.key == key && attempt.value == meta.value);
        if !matches {
            self.violations.push(OracleViolation::PhantomValue {
                key: key.to_string(),
                serial: meta.serial,
                value: meta.value.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use openraft::vote::RaftLeaderId;

    use super::*;

    fn log_id(term: u64, node_id: NodeId, index: u64) -> LogId {
        LogId::new(openraft::impls::leader_id_adv::LeaderId::new(term, node_id), index)
    }

    fn meta(serial: u64, value: &str, log_id: LogId) -> ValueMeta {
        ValueMeta {
            value: value.to_string(),
            serial,
            log_id,
        }
    }

    fn history_with_write(serial: u64, key: &str, value: &str) -> ClientHistory {
        let mut h = ClientHistory::default();
        h.record_write_attempt(serial, key.to_string(), value.to_string());
        h
    }

    #[test]
    fn read_of_attempted_value_is_clean() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None);
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn phantom_value_detected() {
        let mut h = history_with_write(1, "k", "v1");

        // Unknown serial.
        h.record_read("k", Some(&meta(99, "v1", log_id(1, 1, 5))), None);
        // Right serial, wrong value.
        h.record_read("k", Some(&meta(1, "other", log_id(1, 1, 6))), None);
        // Right serial and value, wrong key.
        h.record_read("x", Some(&meta(1, "v1", log_id(1, 1, 7))), None);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 3);
        assert!(violations.iter().all(|v| matches!(v, OracleViolation::PhantomValue { .. })));
    }

    #[test]
    fn monotonic_read_regression_detected() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());

        h.record_read("k", Some(&meta(2, "v2", log_id(2, 1, 9))), None);
        // Older log id observed after a newer one.
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None);
        // Key vanished after being present.
        h.record_read("k", None, None);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 2);
        assert!(violations.iter().all(|v| matches!(v, OracleViolation::NonMonotonicRead { .. })));
    }

    #[test]
    fn monotonic_read_allows_progress() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());

        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None);
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None);
        h.record_read("k", Some(&meta(2, "v2", log_id(2, 2, 9))), None);
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn stale_read_detected() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_acked(1, log_id(2, 1, 9));

        let floor = h.acked_floor("k");
        assert_eq!(floor, Some(log_id(2, 1, 9)));

        // Read started after the ack but observes nothing.
        h.record_read("k", None, floor);
        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1);
        assert!(matches!(&violations[0], OracleViolation::StaleRead { .. }));
    }

    #[test]
    fn read_at_or_above_floor_is_clean() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());
        h.record_write_acked(1, log_id(2, 1, 9));

        let floor = h.acked_floor("k");
        // Exactly the acked write.
        h.record_read("k", Some(&meta(1, "v1", log_id(2, 1, 9))), floor);
        // A later (unacked) write is also fine.
        h.record_read("k", Some(&meta(2, "v2", log_id(2, 1, 12))), floor);
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn unknown_write_outcome_is_never_assumed_absent() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_failed(1);

        // A failed write may still commit and be observed later: not a violation.
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 3))), None);
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn lost_acked_write_detected_in_final_state() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_acked(1, log_id(2, 1, 9));

        // Node 1 lost the key entirely; node 2 has an older log id.
        let mut sm1 = StateMachineData::default();
        sm1.data.insert("other".to_string(), meta(1, "v1", log_id(2, 1, 9)));
        let mut sm2 = StateMachineData::default();
        sm2.data.insert("k".to_string(), meta(1, "v1", log_id(1, 1, 4)));

        h.check_final_durability(&[(1, sm1), (2, sm2)]);
        let violations = h.drain_violations();
        // n1: lost key (+ phantom scan is clean: "other" holds an attempted
        // value but under the wrong key -> also phantom). n2: below the ack.
        assert!(
            violations.iter().any(|v| matches!(v, OracleViolation::LostAckedWrite {
                node: 1,
                found: None,
                ..
            })),
            "{violations:?}"
        );
        assert!(
            violations.iter().any(|v| matches!(v, OracleViolation::LostAckedWrite {
                node: 2,
                found: Some(_),
                ..
            })),
            "{violations:?}"
        );
    }

    #[test]
    fn final_durability_clean_when_overwritten_by_later_write() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());
        h.record_write_acked(1, log_id(2, 1, 9));

        // The acked write was overwritten by a later, never-acked write:
        // that is a legal linearizable history.
        let mut sm = StateMachineData::default();
        sm.data.insert("k".to_string(), meta(2, "v2", log_id(3, 2, 11)));

        h.check_final_durability(&[(1, sm)]);
        assert!(h.drain_violations().is_empty());
    }
}
