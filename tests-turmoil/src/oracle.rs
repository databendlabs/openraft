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
//! - **One serial, one log id**: every request is proposed exactly once, so all sightings of a
//!   serial (ack, read, final scan) must agree on its log id, and one log id must never be seen
//!   carrying two different serials.
//! - **Monotonic reads**: sequential linearizable reads of one key must observe non-decreasing log
//!   ids. Keys are never deleted, so a key can never be observed absent after being observed
//!   present.
//! - **Read-your-writes**: a read that *starts* after a write to the key was acked must observe
//!   that write or a later one.
//! - **Lost acked write** (final): after the cluster heals, every key's value must sit at or above
//!   the highest acked log id for that key.
//! - **Lost observed write** (final): likewise for the highest log id any linearizable read
//!   observed per key — an observation of committed state is a durability commitment even when the
//!   write that produced it was never acked.
//! - **Predecessor chain**: an ack carries the key's previous versioned value, read by the apply at
//!   the state directly below the new entry. It must be the newest known-committed write to the key
//!   below that position — a committed write cannot vanish between two acks, even if no read ever
//!   touches the window.
//! - **Absence floor**: a write applied onto an absent key asserts the key had no committed write
//!   below that log id; a linearizable read observing the key absent asserts none at or below its
//!   `ReadIndex` barrier. No write to the key may ever resolve below such a floor.
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
    ///
    /// This is the only failure outcome on purpose: no `client_write` error
    /// variant proves the entry was never proposed. Even `ForwardToLeader`
    /// is delivered to responders of already-appended entries when their log
    /// is truncated or purged (`RaftCore::run_truncate_log` /
    /// `run_purge_log`), and a purged entry may even have committed — it can
    /// be superseded by a snapshot that contains it. Refining any error into
    /// "definitely absent" would therefore be unsound.
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
    /// After healing, a key ended below a log id that a linearizable read
    /// had already observed (the write itself may never have been acked).
    LostObservedWrite {
        node: NodeId,
        key: String,
        observed: LogId,
        found: Option<LogId>,
    },
    /// One serial was seen at two different log ids: the same request was
    /// applied at two log positions, or an ack pointed at a different entry
    /// than the one later observed.
    DuplicateApply { serial: u64, first: LogId, second: LogId },
    /// One log id was seen carrying two different serials: a single log
    /// position held two different contents.
    DivergentEntry {
        log_id: LogId,
        first_serial: u64,
        second_serial: u64,
    },
    /// An ack's previous-value evidence contradicts the known committed
    /// writes to the key: the apply at `log_id` returned `prev` (`None` =
    /// key absent), but the newest known-committed write to the key below
    /// `log_id` is `newest_below`. Fires when `prev` is not below `log_id`,
    /// when it is older than `newest_below`, or when the key was reported
    /// absent over a committed write.
    BrokenPredecessor {
        key: String,
        log_id: LogId,
        prev: Option<LogId>,
        newest_below: Option<LogId>,
    },
    /// A write was found at a position below an absence floor: an apply's
    /// previous value or an absent read at a `ReadIndex` barrier asserted
    /// that the key holds no committed write below `absent_at` (exclusive).
    WriteBelowAbsence {
        key: String,
        serial: u64,
        log_id: LogId,
        absent_at: LogId,
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
            Self::LostObservedWrite {
                node,
                key,
                observed,
                found,
            } => {
                write!(
                    f,
                    "LostObservedWrite: n{node} key={key} read-observed at {observed}, final state has {found:?}"
                )
            }
            Self::DuplicateApply { serial, first, second } => {
                write!(
                    f,
                    "DuplicateApply: serial={serial} seen at log id {first} and again at {second}"
                )
            }
            Self::DivergentEntry {
                log_id,
                first_serial,
                second_serial,
            } => {
                write!(
                    f,
                    "DivergentEntry: log id {log_id} seen with serial {first_serial} and again with {second_serial}"
                )
            }
            Self::BrokenPredecessor {
                key,
                log_id,
                prev,
                newest_below,
            } => {
                write!(
                    f,
                    "BrokenPredecessor: key={key} apply at {log_id} returned prev {prev:?}, \
                     newest known-committed write below is {newest_below:?}"
                )
            }
            Self::WriteBelowAbsence {
                key,
                serial,
                log_id,
                absent_at,
            } => {
                write!(
                    f,
                    "WriteBelowAbsence: key={key} serial={serial} found at {log_id}, \
                     but the key was asserted absent below {absent_at}"
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
    /// Reads abandoned because no leader was visible to send them to.
    pub reads_no_leader: u64,
    /// Reads whose `ReadIndex` barrier failed or timed out.
    pub reads_failed: u64,
}

impl std::fmt::Display for WorkloadStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "writes attempted/acked/failed={}/{}/{}, reads ok/no-leader/failed={}/{}/{}",
            self.writes_attempted,
            self.writes_acked,
            self.writes_failed,
            self.reads_ok,
            self.reads_no_leader,
            self.reads_failed
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
    /// serial -> log id, pinned by the first evidence (ack or observation).
    resolved_log: BTreeMap<u64, LogId>,
    /// log id -> serial: the reverse direction of [`Self::resolved_log`].
    resolved_serial: BTreeMap<LogId, u64>,
    /// Per key: every exactly-positioned committed write (log id -> serial),
    /// accumulated from all evidence via [`Self::resolve`].
    committed_by_key: BTreeMap<String, BTreeMap<LogId, u64>>,
    /// Per key: the highest log id whose apply observed the key absent.
    /// No write to the key may ever resolve below this floor.
    absent_below: BTreeMap<String, LogId>,
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
    ///
    /// `prev` is the key's previous versioned value returned by the apply
    /// ([`crate::typ::Response::prev`]): an exact observation of the key at
    /// the state directly below `log_id`, checked against the predecessor
    /// chain.
    pub fn record_write_acked(&mut self, serial: u64, log_id: LogId, prev: Option<ValueMeta>) {
        self.stats.writes_acked += 1;
        let Some(attempt) = self.attempts.get_mut(&serial) else {
            return;
        };
        attempt.outcome = WriteOutcome::Acked(log_id);

        let key = attempt.key.clone();
        self.resolve(&key, serial, log_id);
        let is_newer = self.max_acked.get(&key).is_none_or(|(acked, _)| log_id > *acked);
        if is_newer {
            self.max_acked.insert(key.clone(), (log_id, serial));
        }
        self.check_prev_chain(&key, log_id, prev.as_ref());
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

    /// Record a read abandoned before it started: no leader was visible.
    pub fn record_read_no_leader(&mut self) {
        self.stats.reads_no_leader += 1;
    }

    /// Record a read whose `ReadIndex` barrier failed or timed out.
    pub fn record_read_failed(&mut self) {
        self.stats.reads_failed += 1;
    }

    /// Record and check one completed linearizable read.
    ///
    /// `floor` is the value of [`Self::acked_floor`] captured before the read
    /// started. `barrier` is the `ReadIndex` log id returned by
    /// `ensure_linearizable`: the observed state contains at least every
    /// entry at or below it. The state may legitimately have advanced
    /// *beyond* the barrier by the time the value is read, so a present
    /// value asserts nothing extra; but keys are never deleted, so observing
    /// the key *absent* asserts it has no committed write at or below the
    /// barrier.
    pub fn record_read(
        &mut self,
        key: &str,
        observed: Option<&ValueMeta>,
        floor: Option<LogId>,
        barrier: Option<LogId>,
    ) {
        self.stats.reads_ok += 1;

        if let Some(meta) = observed {
            self.check_value_is_attempted(key, meta);
            self.resolve(key, meta.serial, meta.log_id);
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

        // An absent key at a ReadIndex barrier proves the key had no
        // committed write at or below the barrier: check the known committed
        // writes now and raise the key's absence floor for future evidence.
        if observed.is_none()
            && let Some(b) = barrier
        {
            let bound = after(b);
            if let Some((log_id, serial)) =
                self.committed_by_key.get(key).and_then(|writes| writes.range(..bound).next_back())
            {
                self.violations.push(OracleViolation::WriteBelowAbsence {
                    key: key.to_string(),
                    serial: *serial,
                    log_id: *log_id,
                    absent_at: bound,
                });
            }
            self.raise_absence_floor(key, bound);
        }
    }

    /// Check every acked write against the final, healed state machines.
    ///
    /// Call only after convergence: each member must hold, for every key with
    /// an acked write, a value at or above the acked log id, and for every
    /// key a read observed, a value at or above the observed log id — a
    /// linearizable observation of committed state is a durability commitment
    /// exactly like an ack, even when the write itself was never acked.
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
            for (key, observed) in &self.last_read {
                let found = sm.data.get(key);
                let lost = match found {
                    Some(meta) => meta.log_id < *observed,
                    None => true,
                };
                if lost {
                    self.violations.push(OracleViolation::LostObservedWrite {
                        node: *node,
                        key: key.clone(),
                        observed: *observed,
                        found: found.map(|m| m.log_id),
                    });
                }
            }
            for (key, meta) in &sm.data {
                self.check_value_is_attempted(key, meta);
                self.resolve(key, meta.serial, meta.log_id);
            }
        }
    }

    /// Pin `serial` to `log_id` on `key` and check the serial <-> log id
    /// bijection.
    ///
    /// Every evidence source feeds this: write acks, read observations,
    /// previous-value observations and final-scan observations. A request is
    /// proposed exactly once and a log entry has exactly one log id, so the
    /// mapping must be one-to-one in both directions; any disagreement means
    /// an entry was applied twice or an ack pointed at the wrong entry.
    ///
    /// `key` is the key the value was observed under. The position also
    /// feeds the per-key committed-write chain and is checked against the
    /// key's absence floor.
    fn resolve(&mut self, key: &str, serial: u64, log_id: LogId) {
        match self.resolved_log.get(&serial) {
            None => {
                self.resolved_log.insert(serial, log_id);
            }
            Some(first) if *first != log_id => {
                self.violations.push(OracleViolation::DuplicateApply {
                    serial,
                    first: *first,
                    second: log_id,
                });
            }
            Some(_) => {}
        }
        match self.resolved_serial.get(&log_id) {
            None => {
                self.resolved_serial.insert(log_id, serial);
            }
            Some(first) if *first != serial => {
                self.violations.push(OracleViolation::DivergentEntry {
                    log_id,
                    first_serial: *first,
                    second_serial: serial,
                });
            }
            Some(_) => {}
        }
        self.committed_by_key.entry(key.to_string()).or_default().insert(log_id, serial);
        if let Some(absent_at) = self.absent_below.get(key)
            && log_id < *absent_at
        {
            self.violations.push(OracleViolation::WriteBelowAbsence {
                key: key.to_string(),
                serial,
                log_id,
                absent_at: *absent_at,
            });
        }
    }

    /// Check an ack's previous-value evidence against the predecessor chain.
    ///
    /// The apply of the entry at `log_id` read the key's state directly
    /// below its own position, so `prev` must be the newest committed write
    /// to the key below `log_id`. Any known-committed write above `prev`
    /// (or any at all, when `prev` is `None`) vanished from the key without
    /// being overwritten. A `None` also establishes the key's absence floor.
    fn check_prev_chain(&mut self, key: &str, log_id: LogId, prev: Option<&ValueMeta>) {
        if let Some(meta) = prev {
            self.check_value_is_attempted(key, meta);
            self.resolve(key, meta.serial, meta.log_id);
        }

        let newest_below = self
            .committed_by_key
            .get(key)
            .and_then(|writes| writes.range(..log_id).next_back())
            .map(|(l, _)| *l);
        let prev_pos = prev.map(|m| m.log_id);
        let broken = match (prev_pos, newest_below) {
            (Some(p), _) if p >= log_id => true,
            (Some(p), Some(newest)) => p < newest,
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (None, None) => false,
        };
        if broken {
            self.violations.push(OracleViolation::BrokenPredecessor {
                key: key.to_string(),
                log_id,
                prev: prev_pos,
                newest_below,
            });
        }

        if prev.is_none() {
            self.raise_absence_floor(key, log_id);
        }
    }

    /// Raise `key`'s absence floor to `bound`: the key is asserted to hold
    /// no committed write below `bound` (exclusive). Floors only go up.
    fn raise_absence_floor(&mut self, key: &str, bound: LogId) {
        let floor = self.absent_below.entry(key.to_string()).or_insert(bound);
        if *floor < bound {
            *floor = bound;
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

/// The exclusive-bound equivalent of an inclusive one: `x <= b` iff
/// `x < after(b)`. Log indexes are consecutive integers per leader and the
/// leader id orders before the index, so no log id sorts strictly between
/// `(leader, index)` and `(leader, index + 1)`.
fn after(b: LogId) -> LogId {
    LogId::new(b.leader_id, b.index + 1)
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
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None, None);
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn phantom_value_detected() {
        let mut h = history_with_write(1, "k", "v1");

        // Unknown serial.
        h.record_read("k", Some(&meta(99, "v1", log_id(1, 1, 5))), None, None);
        // Right serial, wrong value. (Same log id in both serial-1 reads so
        // only the phantom check fires, not the bijection check.)
        h.record_read("k", Some(&meta(1, "other", log_id(1, 1, 6))), None, None);
        // Right serial and value, wrong key.
        h.record_read("x", Some(&meta(1, "v1", log_id(1, 1, 6))), None, None);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 3);
        assert!(violations.iter().all(|v| matches!(v, OracleViolation::PhantomValue { .. })));
    }

    #[test]
    fn monotonic_read_regression_detected() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());

        h.record_read("k", Some(&meta(2, "v2", log_id(2, 1, 9))), None, None);
        // Older log id observed after a newer one.
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None, None);
        // Key vanished after being present.
        h.record_read("k", None, None, None);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 2);
        assert!(violations.iter().all(|v| matches!(v, OracleViolation::NonMonotonicRead { .. })));
    }

    #[test]
    fn monotonic_read_allows_progress() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());

        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None, None);
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None, None);
        h.record_read("k", Some(&meta(2, "v2", log_id(2, 2, 9))), None, None);
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn stale_read_detected() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_acked(1, log_id(2, 1, 9), None);

        let floor = h.acked_floor("k");
        assert_eq!(floor, Some(log_id(2, 1, 9)));

        // Read started after the ack but observes nothing.
        h.record_read("k", None, floor, None);
        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1);
        assert!(matches!(&violations[0], OracleViolation::StaleRead { .. }));
    }

    #[test]
    fn read_at_or_above_floor_is_clean() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());
        h.record_write_acked(1, log_id(2, 1, 9), None);

        let floor = h.acked_floor("k");
        // Exactly the acked write.
        h.record_read("k", Some(&meta(1, "v1", log_id(2, 1, 9))), floor, None);
        // A later (unacked) write is also fine.
        h.record_read("k", Some(&meta(2, "v2", log_id(2, 1, 12))), floor, None);
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn unknown_write_outcome_is_never_assumed_absent() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_failed(1);

        // A failed write may still commit and be observed later: not a violation.
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 3))), None, None);
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn lost_acked_write_detected_in_final_state() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_acked(1, log_id(2, 1, 9), None);

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
    fn lost_observed_write_detected_in_final_state() {
        // The write was never acked (Unknown), but a linearizable read
        // observed it: it committed, so it must survive to the final state.
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_failed(1);
        h.record_read("k", Some(&meta(1, "v1", log_id(2, 1, 9))), None, None);

        let sm = StateMachineData::default();
        h.check_final_durability(&[(1, sm)]);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::LostObservedWrite {
            node: 1,
            found: None,
            ..
        }));
    }

    #[test]
    fn final_at_or_above_observed_is_clean() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());
        h.record_read("k", Some(&meta(1, "v1", log_id(2, 1, 9))), None, None);

        // Overwritten by a later (never-acked, never-read) write: fine.
        let mut sm = StateMachineData::default();
        sm.data.insert("k".to_string(), meta(2, "v2", log_id(3, 1, 12)));
        h.check_final_durability(&[(1, sm)]);

        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn duplicate_apply_detected_across_ack_and_read() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_acked(1, log_id(2, 1, 9), None);

        // The same serial shows up at a different log position.
        h.record_read("k", Some(&meta(1, "v1", log_id(3, 1, 20))), None, None);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::DuplicateApply {
            serial: 1,
            ..
        }));
    }

    #[test]
    fn duplicate_apply_detected_in_final_scan() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_acked(1, log_id(2, 1, 9), None);

        // Final state holds the acked serial at a higher position: not a
        // lost write, but the same entry applied twice.
        let mut sm = StateMachineData::default();
        sm.data.insert("k".to_string(), meta(1, "v1", log_id(3, 1, 12)));
        h.check_final_durability(&[(1, sm)]);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::DuplicateApply {
            serial: 1,
            ..
        }));
    }

    #[test]
    fn divergent_entry_detected() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "x".to_string(), "v2".to_string());

        // Two different serials observed at the same log id.
        h.record_read("k", Some(&meta(1, "v1", log_id(2, 1, 9))), None, None);
        h.record_read("x", Some(&meta(2, "v2", log_id(2, 1, 9))), None, None);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::DivergentEntry { .. }));
    }

    #[test]
    fn consistent_evidence_is_clean() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_acked(1, log_id(2, 1, 9), None);
        h.record_read("k", Some(&meta(1, "v1", log_id(2, 1, 9))), None, None);

        let mut sm = StateMachineData::default();
        sm.data.insert("k".to_string(), meta(1, "v1", log_id(2, 1, 9)));
        h.check_final_durability(&[(1, sm)]);

        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn prev_chain_consistent_is_clean() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());

        h.record_write_acked(1, log_id(1, 1, 5), None);
        h.record_write_acked(2, log_id(1, 1, 9), Some(meta(1, "v1", log_id(1, 1, 5))));
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn broken_predecessor_missed_committed_write() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());
        h.record_write_attempt(3, "k".to_string(), "v3".to_string());

        h.record_write_acked(1, log_id(1, 1, 5), None);
        h.record_write_acked(2, log_id(1, 1, 9), Some(meta(1, "v1", log_id(1, 1, 5))));
        // W3's apply skipped the committed W2@9: it saw W1@5 as predecessor.
        h.record_write_acked(3, log_id(1, 1, 12), Some(meta(1, "v1", log_id(1, 1, 5))));

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::BrokenPredecessor { .. }));
    }

    #[test]
    fn broken_predecessor_absent_over_committed_write() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());

        h.record_write_acked(1, log_id(1, 1, 5), None);
        // W2's apply saw the key absent although W1 committed below it.
        h.record_write_acked(2, log_id(1, 1, 9), None);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::BrokenPredecessor {
            prev: None,
            newest_below: Some(_),
            ..
        }));
    }

    #[test]
    fn prev_at_or_above_own_position_detected() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());

        // The apply at 9 returned a predecessor from position 10.
        h.record_write_acked(2, log_id(1, 1, 9), Some(meta(1, "v1", log_id(1, 1, 10))));

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::BrokenPredecessor { .. }));
    }

    #[test]
    fn write_below_absence_detected() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());

        // W2's apply at 9 observed the key absent: nothing committed below 9.
        h.record_write_acked(2, log_id(1, 1, 9), None);
        // Yet a read finds W1 committed at 5.
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None, None);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::WriteBelowAbsence {
            serial: 1,
            ..
        }));
    }

    #[test]
    fn absent_read_at_barrier_forbids_later_resolution_below() {
        let mut h = history_with_write(1, "k", "v1");

        // The key is absent at barrier 9: no committed write may exist <= 9.
        h.record_read("k", None, None, Some(log_id(1, 1, 9)));
        // Yet W1 turns up committed at 5.
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None, None);

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::WriteBelowAbsence {
            serial: 1,
            ..
        }));
    }

    #[test]
    fn absent_read_at_barrier_detects_known_write_at_boundary() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_acked(1, log_id(1, 1, 9), None);

        // The barrier covers the acked position itself: <= is violated.
        h.record_read("k", None, None, Some(log_id(1, 1, 9)));

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::WriteBelowAbsence {
            serial: 1,
            ..
        }));
    }

    #[test]
    fn absent_read_without_barrier_is_uninformative() {
        let mut h = history_with_write(1, "k", "v1");

        h.record_read("k", None, None, None);
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 5))), None, None);
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn present_read_beyond_barrier_is_clean() {
        let mut h = history_with_write(1, "k", "v1");

        // The state may have advanced past the barrier by read time.
        h.record_read("k", Some(&meta(1, "v1", log_id(1, 1, 12))), None, Some(log_id(1, 1, 9)));
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn prev_of_unacked_write_is_clean() {
        // W2's prev names the never-acked W1: committed evidence, not a
        // violation; W1's late ack at exactly that position stays consistent.
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());

        h.record_write_acked(2, log_id(1, 1, 9), Some(meta(1, "v1", log_id(1, 1, 5))));
        h.record_write_acked(1, log_id(1, 1, 5), None);
        assert!(h.drain_violations().is_empty());
    }

    #[test]
    fn duplicate_apply_detected_via_prev_value() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());

        h.record_write_acked(1, log_id(1, 1, 5), None);
        // W2's prev carries W1 at position 7, but W1 was acked at 5.
        h.record_write_acked(2, log_id(1, 1, 9), Some(meta(1, "v1", log_id(1, 1, 7))));

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::DuplicateApply {
            serial: 1,
            ..
        }));
    }

    #[test]
    fn phantom_prev_value_detected() {
        let mut h = history_with_write(2, "k", "v2");

        // The apply returned a predecessor no tracked write produced.
        h.record_write_acked(2, log_id(1, 1, 9), Some(meta(99, "vx", log_id(1, 1, 5))));

        let violations = h.drain_violations();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(matches!(&violations[0], OracleViolation::PhantomValue {
            serial: 99,
            ..
        }));
    }

    #[test]
    fn final_durability_clean_when_overwritten_by_later_write() {
        let mut h = history_with_write(1, "k", "v1");
        h.record_write_attempt(2, "k".to_string(), "v2".to_string());
        h.record_write_acked(1, log_id(2, 1, 9), None);

        // The acked write was overwritten by a later, never-acked write:
        // that is a legal linearizable history.
        let mut sm = StateMachineData::default();
        sm.data.insert("k".to_string(), meta(2, "v2", log_id(3, 2, 11)));

        h.check_final_durability(&[(1, sm)]);
        assert!(h.drain_violations().is_empty());
    }
}
