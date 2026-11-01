use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use bench_minimal::network::BenchRaft;
use openraft::ReadPolicy;
use serde_json::Value;
use tokio::sync::Barrier;
use tokio::task::JoinHandle;

use super::BenchConfig;
use super::pace;
use super::percentile;

const READ_YIELD_BATCH: u64 = 64;

type ReadSample = (u64, Result<(), String>);
type ReadTask = JoinHandle<anyhow::Result<Vec<ReadSample>>>;

/// Shares the write clients' start barrier and arrival epoch.
#[derive(Clone)]
pub(super) struct ReadWorkload {
    pub(super) config: Arc<BenchConfig>,
    pub(super) leader: BenchRaft,
    pub(super) start: Arc<Barrier>,
    pub(super) epoch: Arc<OnceLock<Instant>>,
}

impl ReadWorkload {
    pub(super) fn spawn_clients(&self) -> Vec<ReadTask> {
        let mut handles = Vec::new();
        for id in 0..self.config.read_clients {
            let workload = self.clone();
            let handle = tokio::spawn(async move { workload.run_client(id).await });
            handles.push(handle);
        }
        handles
    }

    async fn run_client(&self, id: u64) -> anyhow::Result<Vec<ReadSample>> {
        let operations = client_operations(self.config.read_operations, self.config.read_clients, id);
        let capacity = usize::try_from(operations)?;
        let mut samples = Vec::with_capacity(capacity);
        self.start.wait().await;
        let epoch = *self.epoch.get().context("missing arrival epoch")?;
        for sequence in 0..operations {
            let operation = sequence * self.config.read_clients + id;
            let sample = self.sample(epoch, operation).await?;
            samples.push(sample);
            let yield_due = (sequence + 1).is_multiple_of(READ_YIELD_BATCH);
            if yield_due {
                tokio::task::yield_now().await;
            }
        }
        Ok(samples)
    }

    async fn sample(&self, epoch: Instant, operation: u64) -> anyhow::Result<ReadSample> {
        let started = if self.config.read_rate > 0 {
            pace(epoch, operation, self.config.read_rate).await
        } else {
            Instant::now()
        };
        let result = self.read().await;
        let elapsed = started.elapsed();
        let nanos = elapsed.as_nanos();
        let nanos = u64::try_from(nanos)?;
        let status = match result {
            Ok(()) => Ok(()),
            Err(error) if self.config.allow_read_errors => {
                let message = format!("{error:#}");
                Err(message)
            }
            Err(error) => return Err(error),
        };
        Ok((nanos, status))
    }

    async fn read(&self) -> anyhow::Result<()> {
        if self.config.read_await_ready {
            let read_log_id = self.leader.ensure_linearizable(ReadPolicy::LeaseRead).await?;
            let _ = std::hint::black_box(read_log_id);
        } else {
            let linearizer = self.leader.get_read_linearizer(ReadPolicy::LeaseRead).await?;
            let _ = std::hint::black_box(linearizer);
        }
        Ok(())
    }

    pub(super) fn report(
        &self,
        elapsed: Duration,
        writes: Vec<Vec<u64>>,
        reads: Vec<ReadSample>,
    ) -> anyhow::Result<()> {
        let writes = writes.into_iter().flatten();
        let writes = writes.collect();
        let writes = latency_report(writes, elapsed, self.config.batch_size)?;
        let reads = read_report(reads, elapsed)?;
        let config = serde_json::to_value(self.config.as_ref())?;
        let local_lease_read_enabled = self.local_lease_read_enabled();
        let seconds = elapsed.as_secs_f64();
        let report = serde_json::json!({
            "seconds": seconds, "write": writes, "read": reads,
            "config": config, "local_lease_read_enabled": local_lease_read_enabled,
            "runtime_stats": cfg!(feature = "runtime-stats"),
        });
        println!("{report}");
        Ok(())
    }

    fn local_lease_read_enabled(&self) -> bool {
        let field = self.config.raft_config.get("enable_local_lease_read");
        let enabled = field.and_then(Value::as_bool);
        enabled.unwrap_or(false)
    }
}

fn client_operations(total: u64, clients: u64, id: u64) -> u64 {
    let per_client = total / clients;
    let remainder = total % clients;
    let extra = u64::from(id < remainder);
    per_client + extra
}

fn read_report(samples: Vec<ReadSample>, elapsed: Duration) -> anyhow::Result<Value> {
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    let mut errors = BTreeMap::<String, u64>::new();
    for (nanos, result) in samples {
        match result {
            Ok(()) => successes.push(nanos),
            Err(error) => {
                failures.push(nanos);
                let count = errors.entry(error).or_default();
                *count += 1;
            }
        }
    }
    let successes = latency_report(successes, elapsed, 1)?;
    let failures = latency_report(failures, elapsed, 1)?;
    let report = serde_json::json!({"successful": successes, "failed": failures, "errors": errors});
    Ok(report)
}

fn latency_report(mut samples: Vec<u64>, elapsed: Duration, entries: u64) -> anyhow::Result<Value> {
    samples.sort_unstable();
    let count = samples.len();
    let count = u64::try_from(count)?;
    let operations = count.checked_mul(entries);
    let operations = operations.context("measured operations overflows")?;
    let seconds = elapsed.as_secs_f64();
    let operations_per_second = operations as f64 / seconds;
    let latency = latency(&samples)?;
    let report = serde_json::json!({
        "operations": operations, "operations_per_second": operations_per_second, "latency": latency,
    });
    Ok(report)
}

fn latency(samples: &[u64]) -> anyhow::Result<Value> {
    if samples.is_empty() {
        return Ok(Value::Null);
    }
    let values = samples.iter().map(|value| u128::from(*value));
    let total: u128 = values.sum();
    let count = samples.len();
    let mean = total / count as u128;
    let mean = u64::try_from(mean)?;
    let p50 = percentile(samples, 50);
    let p95 = percentile(samples, 95);
    let p99 = percentile(samples, 99);
    let report = serde_json::json!({
        "samples": count, "mean_ns": mean, "p50_ns": p50, "p95_ns": p95, "p99_ns": p99,
    });
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::client_operations;
    use super::latency_report;
    use super::read_report;

    #[test]
    fn read_failures_have_separate_counts_and_latency() -> anyhow::Result<()> {
        let samples = vec![
            (3, Ok(())),
            (7, Err("expired".to_string())),
            (9, Err("expired".to_string())),
        ];
        let actual = read_report(samples, Duration::from_secs(1))?;
        let expected = serde_json::json!({
            "successful": {"operations": 1, "operations_per_second": 1.0,
                "latency": {"samples": 1, "mean_ns": 3, "p50_ns": 3, "p95_ns": 3, "p99_ns": 3}},
            "failed": {"operations": 2, "operations_per_second": 2.0,
                "latency": {"samples": 2, "mean_ns": 8, "p50_ns": 7, "p95_ns": 9, "p99_ns": 9}},
            "errors": {"expired": 2},
        });
        assert_eq!(expected, actual);
        Ok(())
    }

    #[test]
    fn empty_writes_and_batch_counts_are_reported_exactly() -> anyhow::Result<()> {
        let actual = latency_report(vec![], Duration::from_secs(1), 4)?;
        let expected = serde_json::json!({"operations": 0, "operations_per_second": 0.0, "latency": null});
        assert_eq!(expected, actual);
        let actual = latency_report(vec![10, 20], Duration::from_secs(2), 4)?;
        let expected = serde_json::json!({"operations": 8, "operations_per_second": 4.0,
            "latency": {"samples": 2, "mean_ns": 15, "p50_ns": 10, "p95_ns": 20, "p99_ns": 20}});
        assert_eq!(expected, actual);
        Ok(())
    }

    #[test]
    fn all_requested_reads_are_distributed() {
        let mut actual = Vec::new();
        for id in 0..3 {
            let operations = client_operations(11, 3, id);
            actual.push(operations);
        }
        let expected = vec![4, 4, 3];
        assert_eq!(expected, actual);
    }
}
