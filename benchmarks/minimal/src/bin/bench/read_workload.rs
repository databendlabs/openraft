use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use bench_minimal::network::BenchRaft;
use bench_minimal::store::StateMachineStore;
use bench_minimal::store::TypeConfig;
use openraft::LinearizerOption;
use openraft::ReadPolicy;
use openraft::errors::LinearizableReadError;
use openraft::errors::RaftError;
use openraft::raft::linearizable_read::Linearizer;
use serde_json::Value;
use tokio::sync::Barrier;
use tokio::task::JoinHandle;

use super::BenchConfig;
use super::leader_change::LeaderChange;
use super::leader_change::Leaders;
use super::pace;
use super::percentile;

const READ_YIELD_BATCH: u64 = 64;

/// One read attempt.
pub(super) struct ReadSample {
    /// When the read was due, in nanoseconds since the arrival epoch.
    started_ns: u64,
    latency_ns: u64,
    /// The node that served a successful read.
    node: Option<u64>,
    status: Result<(), String>,
}

type ReadTask = JoinHandle<anyhow::Result<Vec<ReadSample>>>;

/// Shares the write clients' start barrier and arrival epoch.
///
/// The leader authorizes each read; `reader` awaits its barrier and owns `state_machine`, which
/// the query reads. They are the same node unless reads run on a follower. With `leaders`, reads
/// follow leadership instead, and run on whichever node currently leads.
#[derive(Clone)]
pub(super) struct ReadWorkload {
    pub(super) config: Arc<BenchConfig>,
    pub(super) leader: BenchRaft,
    pub(super) reader: BenchRaft,
    pub(super) state_machine: Arc<StateMachineStore>,
    pub(super) leaders: Option<Leaders>,
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
        let latency_ns = u64::try_from(started.elapsed().as_nanos())?;
        let started_ns = u64::try_from(started.saturating_duration_since(epoch).as_nanos())?;
        let (node, status) = match result {
            Ok(node) => (Some(node), Ok(())),
            Err(error) if self.config.allow_read_errors => {
                let message = format!("{error:#}");
                (None, Err(message))
            }
            Err(error) => return Err(error),
        };
        Ok(ReadSample {
            started_ns,
            latency_ns,
            node,
            status,
        })
    }

    /// Runs one read and returns the node that served it.
    async fn read(&self) -> anyhow::Result<u64> {
        let policy = ReadPolicy::from(self.config.read_policy);
        let option = LinearizerOption::from(policy).with_relaxed(self.config.read_relaxed);
        let Some(leaders) = &self.leaders else {
            let linearizer = self.leader.get_local_read_linearizer(option).await?;
            self.finish(linearizer, &self.reader, &self.state_machine).await?;
            let reader = self.config.read_on_follower.unwrap_or(0);
            return Ok(reader);
        };
        loop {
            let (node, raft) = leaders.current();
            let linearizer = match raft.get_local_read_linearizer(option.clone()).await {
                Ok(linearizer) => linearizer,
                Err(error) if is_leader_change(&error) => {
                    leaders.retry_later(node).await;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let machine = leaders.machine(node)?;
            self.finish(linearizer, &raft, machine).await?;
            return Ok(node);
        }
    }

    /// Waits for the barrier on `reader` and queries `machine`, as configured.
    async fn finish(
        &self,
        linearizer: Linearizer<TypeConfig>,
        reader: &BenchRaft,
        machine: &StateMachineStore,
    ) -> anyhow::Result<()> {
        if self.config.read_await_ready || self.config.read_query {
            let state = linearizer.await_ready(reader).await?;
            let _ = std::hint::black_box(state);
        } else {
            let _ = std::hint::black_box(linearizer);
        }
        if self.config.read_query {
            let value = machine.read().await;
            let _ = std::hint::black_box(value);
        }
        Ok(())
    }

    pub(super) fn report(
        &self,
        elapsed: Duration,
        writes: Vec<Vec<u64>>,
        reads: Vec<ReadSample>,
        follower_lag: Option<Value>,
        leader_change: Option<LeaderChange>,
    ) -> anyhow::Result<()> {
        let leader_change = leader_change.map(|change| {
            let served: Vec<_> = reads
                .iter()
                .filter(|sample| sample.status.is_ok())
                .filter_map(|sample| sample.node.map(|node| (sample.started_ns, sample.latency_ns, node)))
                .collect();
            change.report(&served)
        });
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
            "runtime_stats": cfg!(feature = "runtime-stats"), "follower_lag": follower_lag,
            "leader_change": leader_change,
        });
        println!("{report}");
        Ok(())
    }

    fn local_lease_read_enabled(&self) -> bool {
        let config = self.leader.config();
        config.enable_local_lease_read.unwrap_or(false)
    }
}

/// A read rejected because leadership is moving: the node is no longer the leader, is handing
/// leadership over, or has not yet confirmed its leadership with a quorum.
fn is_leader_change(error: &RaftError<TypeConfig, LinearizableReadError<TypeConfig>>) -> bool {
    let quorum_not_enough = matches!(error, RaftError::APIError(LinearizableReadError::QuorumNotEnough(_)));
    error.forward_to_leader().is_some() || quorum_not_enough
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
    for ReadSample { latency_ns, status, .. } in samples {
        match status {
            Ok(()) => successes.push(latency_ns),
            Err(error) => {
                failures.push(latency_ns);
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

    use super::ReadSample;
    use super::client_operations;
    use super::latency_report;
    use super::read_report;

    #[test]
    fn read_failures_have_separate_counts_and_latency() -> anyhow::Result<()> {
        let sample = |latency_ns, status: Result<(), &str>| ReadSample {
            started_ns: 0,
            latency_ns,
            node: status.is_ok().then_some(0),
            status: status.map_err(|error| error.to_string()),
        };
        let samples = vec![sample(3, Ok(())), sample(7, Err("expired")), sample(9, Err("expired"))];
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
