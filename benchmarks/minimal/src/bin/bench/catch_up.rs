//! Cuts one follower off for a while during the run, and measures how it catches up.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use bench_minimal::network::BenchRaft;
use bench_minimal::network::Router;
use bench_minimal::store::TypeConfig;
use openraft::LogIdOptionExt;
use openraft::RaftMetrics;
use openraft::async_runtime::WatchReceiver;
use serde_json::Value;
use tokio::task::JoinHandle;

use super::histogram::Histogram;

/// The follower that is cut off. Node 0 leads, and node 1 keeps a quorum with it.
pub(super) const CUT_NODE: u64 = 2;

/// How long the follower may take to catch up after it is reconnected.
const CATCH_UP_TIMEOUT: Duration = Duration::from_secs(120);

/// At most this many `apply()` calls of the follower are kept for the report.
const CALL_CAPACITY: usize = 1 << 20;

/// Not reached yet: every write that completes before a boundary is set belongs to the phase
/// before it.
const UNSET: u64 = u64::MAX;

/// The phases of the run, in order.
const PHASES: [&str; 4] = ["before", "outage", "catch_up", "after"];

/// When the follower was cut off, reconnected and caught up, and the writes that completed in
/// each phase: before the cut, during the outage, during catch-up, and after it.
pub(super) struct Timeline {
    epoch: Arc<OnceLock<Instant>>,
    cut_ns: AtomicU64,
    reconnected_ns: AtomicU64,
    caught_up_ns: AtomicU64,
    phases: Mutex<[Phase; 4]>,
}

#[derive(Default)]
struct Phase {
    latencies: Histogram,
    last_ns: u64,
}

impl Timeline {
    pub(super) fn new(epoch: Arc<OnceLock<Instant>>) -> Self {
        Self {
            epoch,
            cut_ns: AtomicU64::new(UNSET),
            reconnected_ns: AtomicU64::new(UNSET),
            caught_up_ns: AtomicU64::new(UNSET),
            phases: Mutex::new(Default::default()),
        }
    }

    /// Record a write that completed now, `latency_ns` after it started.
    pub(super) fn record(&self, latency_ns: u64) {
        let Some(epoch) = self.epoch.get() else {
            return;
        };
        let at = nanos_since(*epoch);
        let phase = phase_of(at, [
            self.cut_ns.load(Ordering::Acquire),
            self.reconnected_ns.load(Ordering::Acquire),
            self.caught_up_ns.load(Ordering::Acquire),
        ]);
        let mut phases = self.phases.lock().unwrap();
        let phase = &mut phases[phase];
        phase.latencies.record(latency_ns);
        phase.last_ns = phase.last_ns.max(at);
    }

    /// Report the writes of every phase. Call it after the last write: writes keep completing in
    /// the last phase after the follower has caught up.
    pub(super) fn report(&self) -> Value {
        let boundaries = [
            0,
            self.cut_ns.load(Ordering::Acquire),
            self.reconnected_ns.load(Ordering::Acquire),
            self.caught_up_ns.load(Ordering::Acquire),
        ];
        let phases = self.phases.lock().unwrap();
        let mut report = serde_json::Map::new();
        for (index, name) in PHASES.iter().enumerate() {
            let phase = &phases[index];
            let start = boundaries[index];
            let end = match boundaries.get(index + 1) {
                Some(end) if *end != UNSET => *end,
                _ => phase.last_ns,
            };
            report.insert(name.to_string(), phase_report(phase, start, end));
        }
        Value::Object(report)
    }
}

/// The index in [`PHASES`] of a write that completed `at`, given the boundaries that have been
/// reached: the cut, the reconnection and the catch-up.
fn phase_of(at: u64, boundaries: [u64; 3]) -> usize {
    boundaries.iter().take_while(|boundary| at >= **boundary).count()
}

fn phase_report(phase: &Phase, start_ns: u64, end_ns: u64) -> Value {
    let writes = phase.latencies.count;
    if writes == 0 || start_ns == UNSET || end_ns <= start_ns {
        return serde_json::json!({"writes": writes});
    }
    let seconds = (end_ns - start_ns) as f64 / 1e9;
    serde_json::json!({"writes":writes,"seconds":seconds,"writes_per_second":writes as f64 / seconds,
        "p50_ns":phase.latencies.percentile(50),"p99_ns":phase.latencies.percentile(99),"max_ns":phase.latencies.percentile(100)})
}

/// The follower's `apply()` calls from its reconnection on.
#[derive(Default)]
struct Calls {
    recording: bool,
    /// When each call published its result, and how many entries it applied.
    calls: Vec<(Instant, u64)>,
    dropped: u64,
}

/// The follower's state, as its metrics report it.
#[derive(Clone, Copy, Debug)]
struct FollowerState {
    last_log: u64,
    committed: u64,
    applied: u64,
    snapshot: u64,
}

impl FollowerState {
    fn read(raft: &BenchRaft) -> Self {
        let metrics = raft.metrics();
        let metrics = metrics.borrow_watched();
        Self::of(&metrics)
    }

    fn of(metrics: &RaftMetrics<TypeConfig>) -> Self {
        Self {
            last_log: metrics.last_log_index.unwrap_or(0),
            committed: metrics.local_committed.index().unwrap_or(0),
            applied: metrics.last_applied.index().unwrap_or(0),
            snapshot: metrics.snapshot.index().unwrap_or(0),
        }
    }
}

/// Cuts [`CUT_NODE`] off `after` the epoch, reconnects it `outage` later, and waits until it
/// has applied every entry the leader had committed by then.
pub(super) fn spawn(
    router: Router,
    timeline: Arc<Timeline>,
    epoch: Instant,
    after: Duration,
    outage: Duration,
) -> JoinHandle<anyhow::Result<Value>> {
    tokio::spawn(async move {
        let leader = router.get_raft(0);
        let follower = router.get_raft(CUT_NODE);
        let calls = Arc::new(Mutex::new(Calls::default()));
        let recorded = calls.clone();
        let machine = follower.with_state_machine(|sm| Box::pin(async move { sm.clone() })).await?;
        machine
            .observe_applies(Arc::new(move |_index, entries, at| {
                let mut calls = recorded.lock().unwrap();
                if !calls.recording {
                    return;
                }
                if calls.calls.len() == CALL_CAPACITY {
                    calls.dropped += 1;
                    return;
                }
                calls.calls.push((at, entries));
            }))
            .map_err(anyhow::Error::msg)?;

        tokio::time::sleep_until(tokio::time::Instant::from_std(epoch + after)).await;
        let cut_ns = nanos_since(epoch);
        router.cut_off(CUT_NODE);
        timeline.cut_ns.store(cut_ns, Ordering::Release);
        let at_cut = FollowerState::read(&follower);

        tokio::time::sleep(outage).await;
        let (target, leader_purged) = {
            let metrics = leader.metrics();
            let metrics = metrics.borrow_watched();
            let committed = metrics.local_committed.index().context("the leader has committed no entry")?;
            (committed, metrics.purged.index())
        };
        let at_reconnect = FollowerState::read(&follower);
        calls.lock().unwrap().recording = true;
        let reconnected = Instant::now();
        let reconnected_ns = nanos_since(epoch);
        router.reconnect(CUT_NODE);
        timeline.reconnected_ns.store(reconnected_ns, Ordering::Release);

        let progress = tokio::time::timeout(CATCH_UP_TIMEOUT, catch_up(&follower, reconnected, target)).await;
        let caught_up_ns = nanos_since(epoch);
        let progress = match progress {
            Ok(progress) => {
                timeline.caught_up_ns.store(caught_up_ns, Ordering::Release);
                Some(progress?)
            }
            Err(_) => None,
        };
        let at_catch_up = FollowerState::read(&follower);
        let calls = {
            let mut calls = calls.lock().unwrap();
            calls.recording = false;
            std::mem::take(&mut *calls)
        };

        let mut entries = Histogram::default();
        let mut call_count = 0u64;
        let caught_up = progress.as_ref().map(|progress| reconnected + progress.applied);
        for (at, applied) in &calls.calls {
            if caught_up.is_none_or(|caught_up| *at <= caught_up) {
                entries.record(*applied);
                call_count += 1;
            }
        }
        let progress = match &progress {
            Some(progress) => serde_json::json!({"status":"caught_up",
                "first_entry_ns":progress.first_entry.as_nanos() as u64,
                "replicated_ns":progress.replicated.as_nanos() as u64,
                "committed_ns":progress.committed.as_nanos() as u64,
                "applied_ns":progress.applied.as_nanos() as u64,
                "max_committed_unapplied":progress.max_committed_unapplied,
                "max_logged_unapplied":progress.max_logged_unapplied}),
            None => serde_json::json!({"status":"timed_out","timeout_ns":CATCH_UP_TIMEOUT.as_nanos() as u64}),
        };
        let mut report = serde_json::json!({"node":CUT_NODE,"cut_ns":cut_ns,"reconnected_ns":reconnected_ns,
            "outage_ns":reconnected_ns - cut_ns,"leader_committed_at_reconnect":target,
            "leader_purged_at_reconnect":leader_purged,
            "entries_behind_at_reconnect":target.saturating_sub(at_reconnect.applied),
            "follower":{"at_cut":state_report(at_cut),"at_reconnect":state_report(at_reconnect),
                "at_catch_up":state_report(at_catch_up)},
            "apply_calls_until_caught_up":{"calls":call_count,"entries":entries.report(),"dropped":calls.dropped}});
        report["catch_up"] = progress;
        Ok(report)
    })
}

fn state_report(state: FollowerState) -> Value {
    serde_json::json!({"last_log":state.last_log,"committed":state.committed,"applied":state.applied,
        "snapshot":state.snapshot})
}

/// How long after its reconnection the follower first reported a new entry in its log, then
/// `target` in its log, as committed, and as applied, and how far its apply trailed its log in
/// the meantime.
struct Progress {
    first_entry: Duration,
    replicated: Duration,
    committed: Duration,
    applied: Duration,
    max_committed_unapplied: u64,
    max_logged_unapplied: u64,
}

async fn catch_up(follower: &BenchRaft, reconnected: Instant, target: u64) -> anyhow::Result<Progress> {
    let mut metrics = follower.metrics();
    let start = FollowerState::of(&metrics.borrow_watched()).last_log;
    let mut first_entry = None;
    let mut replicated = None;
    let mut committed = None;
    let mut max_committed_unapplied = 0;
    let mut max_logged_unapplied = 0;
    loop {
        let state = FollowerState::of(&metrics.borrow_watched());
        let now = reconnected.elapsed();
        max_committed_unapplied = max_committed_unapplied.max(state.committed.saturating_sub(state.applied));
        max_logged_unapplied = max_logged_unapplied.max(state.last_log.saturating_sub(state.applied));
        if first_entry.is_none() && state.last_log > start {
            first_entry = Some(now);
        }
        if replicated.is_none() && state.last_log >= target {
            replicated = Some(now);
        }
        if committed.is_none() && state.committed >= target {
            committed = Some(now);
        }
        if state.applied >= target {
            return Ok(Progress {
                first_entry: first_entry.unwrap_or(now),
                replicated: replicated.unwrap_or(now),
                committed: committed.unwrap_or(now),
                applied: now,
                max_committed_unapplied,
                max_logged_unapplied,
            });
        }
        metrics.changed().await?;
    }
}

fn nanos_since(epoch: Instant) -> u64 {
    u64::try_from(epoch.elapsed().as_nanos()).unwrap_or(u64::MAX - 1)
}

#[cfg(test)]
mod tests {
    use super::UNSET;
    use super::phase_of;

    #[test]
    fn a_write_belongs_to_the_last_boundary_it_completed_after() {
        let none = [UNSET, UNSET, UNSET];
        assert_eq!(0, phase_of(5, none));

        let cut = [10, UNSET, UNSET];
        assert_eq!(0, phase_of(9, cut));
        assert_eq!(1, phase_of(10, cut));

        let all = [10, 20, 30];
        assert_eq!(1, phase_of(19, all));
        assert_eq!(2, phase_of(20, all));
        assert_eq!(2, phase_of(29, all));
        assert_eq!(3, phase_of(30, all));
    }
}
