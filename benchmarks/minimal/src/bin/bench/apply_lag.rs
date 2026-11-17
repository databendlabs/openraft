use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Instant;

use bench_minimal::network::BenchRaft;
use openraft::LogIdOptionExt;
use openraft::async_runtime::WatchReceiver;
use serde_json::Value;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::measurement::Bounds;

/// Applied-index changes of one node: nanoseconds since recording started, and the index.
type Trace = VecDeque<(u64, u64)>;
const TRACE_CAPACITY: usize = 4096;

/// Records the applied log indexes the leader and one follower publish in their metrics, to
/// report how far the follower's apply trails the leader's.
pub(super) struct ApplyLag {
    stop: watch::Sender<bool>,
    leader: JoinHandle<anyhow::Result<(Trace, u64)>>,
    follower: JoinHandle<anyhow::Result<(Trace, u64)>>,
}

impl ApplyLag {
    pub(super) fn start(
        leader: &BenchRaft,
        follower: &BenchRaft,
        epoch: Arc<OnceLock<Instant>>,
        bounds: Bounds,
    ) -> Self {
        let (stop, stopped) = watch::channel(false);
        let leader = record(leader.clone(), epoch.clone(), bounds, stopped.clone());
        let follower = record(follower.clone(), epoch, bounds, stopped);
        Self { stop, leader, follower }
    }

    /// Stops recording and summarizes the follower's lag.
    pub(super) async fn finish(self) -> anyhow::Result<Value> {
        self.stop.send_replace(true);
        let (leader, leader_dropped) = self.leader.await??;
        let (follower, follower_dropped) = self.follower.await??;
        let leader: Vec<_> = leader.into_iter().collect();
        let follower: Vec<_> = follower.into_iter().collect();
        let mut report = lag_report(&leader, &follower);
        report["trace_sampling"] = serde_json::json!({"capacity_per_node":TRACE_CAPACITY,"leader_dropped":leader_dropped,"follower_dropped":follower_dropped,
            "scope":"retained_recent_metrics_watch_observations_in_measurement"});
        Ok(report)
    }
}

fn record(
    raft: BenchRaft,
    epoch: Arc<OnceLock<Instant>>,
    bounds: Bounds,
    mut stopped: watch::Receiver<bool>,
) -> JoinHandle<anyhow::Result<(Trace, u64)>> {
    tokio::spawn(async move {
        let mut metrics = raft.metrics();
        let mut trace = Trace::new();
        let mut dropped = 0u64;
        loop {
            let applied = metrics.borrow_watched().last_applied.index().unwrap_or(0);
            if let Some(origin) = epoch.get() {
                let nanos = origin.elapsed().as_nanos() as u64;
                if bounds.contains(nanos) && trace.back().is_none_or(|(_, index)| *index != applied) {
                    if trace.len() == TRACE_CAPACITY {
                        trace.pop_front();
                        dropped += 1;
                    }
                    trace.push_back((nanos, applied));
                }
            }
            tokio::select! { changed=metrics.changed()=>changed?,_=stopped.changed()=>break }
        }
        Ok((trace, dropped))
    })
}

/// For every applied index the follower reports, how many entries the leader had applied beyond
/// it at that time, and how long after the leader first reported that index the follower did.
///
/// Negative values mean the follower was ahead of the leader.
fn lag_report(leader: &[(u64, u64)], follower: &[(u64, u64)]) -> Value {
    let mut entries = Vec::new();
    let mut nanos = Vec::new();
    let mut entries_omitted = 0u64;
    let mut times_omitted = 0u64;
    for &(at, index) in follower {
        let seen = leader.partition_point(|(time, _)| *time <= at);
        if let Some(last) = seen.checked_sub(1) {
            entries.push(signed(leader[last].1) - signed(index));
        } else {
            entries_omitted += 1;
        }
        let retained = leader.first().is_some_and(|(_, first)| index >= *first);
        let reached = leader.partition_point(|(_, applied)| *applied < index);
        if retained {
            if let Some((time, _)) = leader.get(reached) {
                nanos.push(signed(at) - signed(*time));
                continue;
            }
        }
        times_omitted += 1;
    }
    serde_json::json!({"entries_behind":summary(entries),"time_behind_ns":summary(nanos),
        "entries_samples_omitted":entries_omitted,"time_samples_omitted":times_omitted})
}

/// Converts to `i64`, saturating; indexes and nanoseconds of a run stay far below the limit.
fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn summary(mut samples: Vec<i64>) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    samples.sort_unstable();
    let rank = |percentile: usize| samples[(samples.len() * percentile).div_ceil(100) - 1];
    let (p50, p99) = (rank(50), rank(99));
    let (min, max) = (samples[0], samples[samples.len() - 1]);
    serde_json::json!({"samples": samples.len(), "min": min, "p50": p50, "p99": p99, "max": max})
}

#[cfg(test)]
mod tests {
    use super::lag_report;

    #[test]
    fn a_follower_behind_the_leader_has_positive_lag() {
        let leader = [(0, 10), (100, 12), (200, 15)];
        let follower = [(0, 10), (150, 12), (250, 13), (260, 15)];
        let actual = lag_report(&leader, &follower);
        let expected = serde_json::json!({
            "entries_samples_omitted":0,"time_samples_omitted":0,
            "entries_behind": {"samples": 4, "min": 0, "p50": 0, "p99": 2, "max": 2},
            "time_behind_ns": {"samples": 4, "min": 0, "p50": 50, "p99": 60, "max": 60},
        });
        assert_eq!(expected, actual);
    }

    #[test]
    fn a_follower_ahead_of_the_leader_has_negative_lag() {
        let leader = [(0, 10), (300, 12)];
        let follower = [(0, 10), (100, 12)];
        let actual = lag_report(&leader, &follower);
        let expected = serde_json::json!({
            "entries_samples_omitted":0,"time_samples_omitted":0,
            "entries_behind": {"samples": 2, "min": -2, "p50": -2, "p99": 0, "max": 0},
            "time_behind_ns": {"samples": 2, "min": -200, "p50": -200, "p99": 0, "max": 0},
        });
        assert_eq!(expected, actual);
    }

    #[test]
    fn indexes_the_leader_never_reported_have_no_time_lag() {
        let leader = [(0, 10)];
        let follower = [(0, 10), (50, 11)];
        let actual = lag_report(&leader, &follower);
        let expected = serde_json::json!({
            "entries_samples_omitted":0,"time_samples_omitted":1,
            "entries_behind": {"samples": 2, "min": -1, "p50": -1, "p99": 0, "max": 0},
            "time_behind_ns": {"samples": 1, "min": 0, "p50": 0, "p99": 0, "max": 0},
        });
        assert_eq!(expected, actual);
    }
    #[test]
    fn nonoverlapping_retained_prefixes_are_omitted() {
        let leader = [(100, 50), (200, 60)];
        let follower = [(90, 40), (150, 45), (250, 60)];
        let actual = lag_report(&leader, &follower);
        let expected = serde_json::json!({"entries_behind":{"samples":2,"min":0,"p50":0,"p99":5,"max":5},
            "time_behind_ns":{"samples":1,"min":50,"p50":50,"p99":50,"max":50},
            "entries_samples_omitted":1,"time_samples_omitted":2});
        assert_eq!(expected, actual);
    }
}
