use std::time::Instant;

use bench_minimal::network::BenchRaft;
use openraft::LogIdOptionExt;
use openraft::async_runtime::WatchReceiver;
use serde_json::Value;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Applied-index changes of one node: nanoseconds since recording started, and the index.
type Trace = Vec<(u64, u64)>;

/// Records the applied log indexes the leader and one follower publish in their metrics, to
/// report how far the follower's apply trails the leader's.
pub(super) struct ApplyLag {
    stop: watch::Sender<bool>,
    leader: JoinHandle<anyhow::Result<Trace>>,
    follower: JoinHandle<anyhow::Result<Trace>>,
}

impl ApplyLag {
    pub(super) fn start(leader: &BenchRaft, follower: &BenchRaft) -> Self {
        let (stop, stopped) = watch::channel(false);
        let origin = Instant::now();
        let leader = record(leader.clone(), origin, stopped.clone());
        let follower = record(follower.clone(), origin, stopped);
        Self { stop, leader, follower }
    }

    /// Stops recording and summarizes the follower's lag.
    pub(super) async fn finish(self) -> anyhow::Result<Value> {
        self.stop.send_replace(true);
        let leader = self.leader.await??;
        let follower = self.follower.await??;
        Ok(lag_report(&leader, &follower))
    }
}

fn record(raft: BenchRaft, origin: Instant, mut stopped: watch::Receiver<bool>) -> JoinHandle<anyhow::Result<Trace>> {
    tokio::spawn(async move {
        let mut metrics = raft.metrics();
        let mut trace = Trace::new();
        loop {
            let applied = metrics.borrow_watched().last_applied.index().unwrap_or(0);
            if trace.last().map(|(_, index)| *index) != Some(applied) {
                let nanos = u64::try_from(origin.elapsed().as_nanos())?;
                trace.push((nanos, applied));
            }
            tokio::select! {
                changed = metrics.changed() => changed?,
                _ = stopped.changed() => break,
            }
        }
        Ok(trace)
    })
}

/// For every applied index the follower reports, how many entries the leader had applied beyond
/// it at that time, and how long after the leader first reported that index the follower did.
///
/// Negative values mean the follower was ahead of the leader.
fn lag_report(leader: &[(u64, u64)], follower: &[(u64, u64)]) -> Value {
    let mut entries = Vec::new();
    let mut nanos = Vec::new();
    for &(at, index) in follower {
        let seen = leader.partition_point(|(time, _)| *time <= at);
        let leader_index = seen.checked_sub(1).map_or(0, |last| leader[last].1);
        entries.push(signed(leader_index) - signed(index));
        let reached = leader.partition_point(|(_, applied)| *applied < index);
        if let Some((time, _)) = leader.get(reached) {
            nanos.push(signed(at) - signed(*time));
        }
    }
    serde_json::json!({"entries_behind": summary(entries), "time_behind_ns": summary(nanos)})
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
            "entries_behind": {"samples": 2, "min": -1, "p50": -1, "p99": 0, "max": 0},
            "time_behind_ns": {"samples": 1, "min": 0, "p50": 0, "p99": 0, "max": 0},
        });
        assert_eq!(expected, actual);
    }
}
