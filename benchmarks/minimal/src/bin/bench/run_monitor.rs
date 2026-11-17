use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use serde_json::Value;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::measurement::Bounds;

const INTERVAL: Duration = Duration::from_millis(100);
const MAX_POINTS: usize = 4096;

#[derive(Default)]
pub(super) struct Live {
    offered: AtomicU64,
    admitted: AtomicU64,
    finished: AtomicU64,
}

impl Live {
    pub(super) fn offer(&self, admitted: bool) {
        self.offered.fetch_add(1, Ordering::Relaxed);
        if admitted {
            self.admitted.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(super) fn finish(&self) {
        self.finished.fetch_add(1, Ordering::Relaxed);
    }
    fn snapshot(&self) -> Value {
        let offered = self.offered.load(Ordering::Relaxed);
        let admitted = self.admitted.load(Ordering::Relaxed);
        let finished = self.finished.load(Ordering::Relaxed);
        serde_json::json!({"offered_requests":offered,"admitted_requests":admitted,
            "finished_requests":finished,"pending_requests":admitted.saturating_sub(finished)})
    }
}

pub(super) struct RunMonitor {
    stop: watch::Sender<bool>,
    backlog: JoinHandle<Value>,
    resources: JoinHandle<std::io::Result<Value>>,
}

impl RunMonitor {
    pub(super) fn start(epoch: Instant, bounds: Bounds, writes: Arc<Live>, reads: Arc<Live>) -> Self {
        let (stop, mut stopped) = watch::channel(false);
        let backlog = tokio::spawn(async move {
            let mut samples = Vec::new();
            let mut interval = tokio::time::interval(INTERVAL);
            let mut dropped = 0u64;
            loop {
                tokio::select! { _=interval.tick()=>{}, _=stopped.changed()=>break }
                let at = epoch.elapsed().as_nanos() as u64;
                if !bounds.contains(at) {
                    continue;
                }
                if samples.len() < MAX_POINTS {
                    samples.push(serde_json::json!({"at_ns":at,"write":writes.snapshot(),"read":reads.snapshot()}));
                } else {
                    dropped += 1;
                }
            }
            serde_json::json!({"samples":samples,"samples_dropped":dropped,"sample_capacity":MAX_POINTS,"consistency":"approximate_independent_atomic_loads"})
        });
        let resources = tokio::spawn(measure_resources(epoch, bounds));
        Self {
            stop,
            backlog,
            resources,
        }
    }

    pub(super) async fn finish(self, operations: u64) -> anyhow::Result<Value> {
        self.stop.send_replace(true);
        let backlog = self.backlog.await?;
        let mut resources = self.resources.await??;
        if let Some(cpu) = resources["cpu_seconds"].as_f64() {
            resources["cpu_ns_per_completed_operation"] = if operations == 0 {
                Value::Null
            } else {
                serde_json::json!(cpu * 1e9 / operations as f64)
            };
        }
        Ok(serde_json::json!({"client_backlog":backlog,"resources":resources}))
    }
}

async fn measure_resources(epoch: Instant, bounds: Bounds) -> std::io::Result<Value> {
    let Some(end) = bounds.end else {
        return Ok(Value::Null);
    };
    tokio::time::sleep_until(tokio::time::Instant::from_std(epoch + bounds.start)).await;
    let started = epoch.elapsed();
    let before = usage()?;
    tokio::time::sleep_until(tokio::time::Instant::from_std(epoch + end)).await;
    let finished = epoch.elapsed();
    let after = usage()?;
    Ok(
        serde_json::json!({"cpu_seconds":after.0-before.0,"process_peak_rss_bytes_at_end":after.1,
        "sample_start_ns":started.as_nanos(),"sample_end_ns":finished.as_nanos(),
        "scope":"whole_process_at_measurement_boundaries_before_json_reporting",
        "rss_scope":"high_water_mark_since_process_start"}),
    )
}

#[cfg(unix)]
pub(super) fn usage() -> std::io::Result<(f64, u64)> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let usage = unsafe { usage.assume_init() };
    let user = usage.ru_utime.tv_sec as f64 + usage.ru_utime.tv_usec as f64 / 1e6;
    let system = usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1e6;
    let rss = usage.ru_maxrss as u64;
    let rss = if cfg!(target_os = "macos") { rss } else { rss * 1024 };
    Ok((user + system, rss))
}

#[cfg(not(unix))]
pub(super) fn usage() -> std::io::Result<(f64, u64)> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "measurement resource sampling requires getrusage",
    ))
}

#[cfg(feature = "runtime-stats")]
pub(super) fn command_delta(before: &[u64], after: &[u64]) -> Value {
    let counts: std::collections::BTreeMap<_, _> = openraft::stats::CommandName::ALL
        .iter()
        .map(|command| {
            let index = command.index();
            (command.as_str(), after[index].saturating_sub(before[index]))
        })
        .collect();
    serde_json::json!({"counts":counts})
}

#[cfg(feature = "runtime-stats")]
pub(super) async fn command_window(
    leader: bench_minimal::network::BenchRaft,
    epoch: Instant,
    bounds: Bounds,
) -> anyhow::Result<Value> {
    let end = bounds.end.expect("command window requires timed measurement");
    tokio::time::sleep_until(tokio::time::Instant::from_std(epoch + bounds.start)).await;
    let before = leader.runtime_stats().await?.command_counts;
    let start_observed = epoch.elapsed();
    tokio::time::sleep_until(tokio::time::Instant::from_std(epoch + end)).await;
    let after = leader.runtime_stats().await?.command_counts;
    let end_observed = epoch.elapsed();
    let mut report = command_delta(&before, &after);
    report["scope"] = serde_json::json!("core_snapshots_requested_at_measurement_boundaries");
    report["start_observed_ns"] = serde_json::json!(start_observed.as_nanos());
    report["end_observed_ns"] = serde_json::json!(end_observed.as_nanos());
    Ok(report)
}
