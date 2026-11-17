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

use super::BenchConfig;
use super::arrival::Arrival;
use super::arrival::Handler;
use super::arrival::Task;
use super::arrival::Workload;
use super::leader_change::LeaderChange;
use super::leader_change::Leaders;
use super::request_sample::Outcome;
use super::request_sample::RequestSample;
use super::request_sample::latency_report;
use super::request_sample::nanos;
use super::request_sample::timing_report;
use super::request_sample::with_deadline;

pub(super) type ReadSample = RequestSample;

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
    pub(super) fn spawn_clients(&self) -> Vec<Task> {
        let workload = Workload {
            config: self.config.arrival.clone(),
            clients: self.config.read_clients,
            operations: self.config.read_operations,
            rate: self.config.read_rate as f64,
            measured: true,
            stream_seed: super::arrival::READ_STREAM,
            start: self.start.clone(),
            epoch: self.epoch.clone(),
        };
        workload.spawn(self.clone())
    }

    async fn sample(&self, arrival: Arrival) -> anyhow::Result<ReadSample> {
        let epoch = *self.epoch.get().context("missing arrival epoch")?;
        let every = self.config.arrival.phase_sample_every;
        let sampled = every > 0 && arrival.operation.is_multiple_of(every);
        let mut sample = RequestSample::new(epoch, arrival, sampled)?;
        if self.leaders.is_some() {
            sample.invoked_ns = Some(nanos(epoch, Instant::now())?);
        }
        let future = self.read(epoch, &mut sample);
        let timeout = self.config.arrival.request_timeout_ms;
        let result = with_deadline(future, arrival.scheduled, timeout).await;
        self.read_outcome(&mut sample, result)?;
        sample.finish(epoch, arrival.scheduled)?;
        Ok(sample)
    }

    fn read_outcome(&self, sample: &mut ReadSample, result: anyhow::Result<Option<u64>>) -> anyhow::Result<()> {
        match result {
            Ok(Some(node)) => sample.node = Some(node),
            Ok(None) => sample.outcome = Outcome::TimedOut,
            Err(error) if self.config.allow_read_errors || self.config.arrival.open_loop => {
                sample.outcome = Outcome::Failed(format!("{error:#}"));
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    /// Runs one read and returns the node that served it.
    async fn read(&self, epoch: Instant, sample: &mut ReadSample) -> anyhow::Result<u64> {
        if self.config.arrival.arrival_noop {
            return Ok(0);
        }
        let policy = ReadPolicy::from(self.config.read_policy);
        let option = LinearizerOption::from(policy).with_relaxed(self.config.read_relaxed);
        let Some(leaders) = &self.leaders else {
            let linearizer = self.leader.get_local_read_linearizer(option).await?;
            sample.mark_authorized(epoch)?;
            self.finish(linearizer, &self.reader, &self.state_machine, epoch, sample).await?;
            let reader = self.config.read_on_follower.unwrap_or(0);
            return Ok(reader);
        };
        loop {
            let (node, raft) = leaders.current();
            let linearizer = match raft.get_local_read_linearizer(option.clone()).await {
                Ok(linearizer) => linearizer,
                Err(error) if is_leader_change(&error) => {
                    sample.retries += 1;
                    leaders.retry_later(node).await;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let machine = leaders.machine(node)?;
            sample.mark_authorized(epoch)?;
            self.finish(linearizer, &raft, machine, epoch, sample).await?;
            return Ok(node);
        }
    }

    /// Waits for the barrier on `reader` and queries `machine`, as configured.
    async fn finish(
        &self,
        linearizer: Linearizer<TypeConfig>,
        reader: &BenchRaft,
        machine: &StateMachineStore,
        epoch: Instant,
        sample: &mut ReadSample,
    ) -> anyhow::Result<()> {
        if self.config.read_await_ready || self.config.read_query {
            let state = linearizer.await_ready(reader).await?;
            let _ = std::hint::black_box(state);
        } else {
            let _ = std::hint::black_box(linearizer);
        }
        sample.mark_barrier(epoch)?;
        if self.config.read_query {
            if sample.phases.is_some() {
                let (value, locked) = machine.read_timed().await;
                sample.mark_query_lock(epoch, locked)?;
                let _ = std::hint::black_box(value);
            } else {
                let value = machine.read().await;
                let _ = std::hint::black_box(value);
            }
        }
        Ok(())
    }

    pub(super) fn report(
        &self,
        elapsed: Duration,
        writes: Vec<Vec<RequestSample>>,
        reads: Vec<ReadSample>,
        follower_lag: Option<Value>,
        leader_change: Option<LeaderChange>,
    ) -> anyhow::Result<()> {
        let leader_change = leader_change.map(|change| {
            let served: Vec<_> = reads
                .iter()
                .filter(|sample| sample.outcome == Outcome::Completed)
                .filter_map(|sample| {
                    let node = sample.node?;
                    let invoked = sample.invoked_ns?;
                    let completed = sample.scheduled_ns + sample.latency_ns;
                    Some((invoked, completed.saturating_sub(invoked), node))
                })
                .collect();
            change.report(&served)
        });
        let writes = writes.into_iter().flatten();
        let writes: Vec<RequestSample> = writes.collect();
        let write_timing = timing_report(&writes, self.config.batch_size)?;
        let completed = writes.iter().filter(|sample| sample.outcome == Outcome::Completed);
        let latencies = completed.map(|sample| sample.latency_ns).collect();
        let writes = latency_report(latencies, elapsed, self.config.batch_size)?;
        let reads = read_report(reads, elapsed)?;
        let config = serde_json::to_value(self.config.as_ref())?;
        let local_lease_read_enabled = self.local_lease_read_enabled();
        let seconds = elapsed.as_secs_f64();
        let report = serde_json::json!({
            "seconds": seconds, "write": writes, "read": reads, "write_timing": write_timing,
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

impl Handler for ReadWorkload {
    async fn run(&self, arrival: Arrival) -> anyhow::Result<Option<RequestSample>> {
        let sample = self.sample(arrival).await?;
        Ok(Some(sample))
    }
}

/// A read rejected because leadership is moving: the node is no longer the leader, is handing
/// leadership over, or has not yet confirmed its leadership with a quorum.
fn is_leader_change(error: &RaftError<TypeConfig, LinearizableReadError<TypeConfig>>) -> bool {
    let quorum_not_enough = matches!(error, RaftError::APIError(LinearizableReadError::QuorumNotEnough(_)));
    error.forward_to_leader().is_some() || quorum_not_enough
}

fn read_report(samples: Vec<ReadSample>, elapsed: Duration) -> anyhow::Result<Value> {
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    let mut errors = BTreeMap::<String, u64>::new();
    let timing = timing_report(&samples, 1)?;
    for ReadSample {
        latency_ns, outcome, ..
    } in samples
    {
        match outcome {
            Outcome::Completed => successes.push(latency_ns),
            Outcome::Failed(error) => {
                failures.push(latency_ns);
                let count = errors.entry(error).or_default();
                *count += 1;
            }
            Outcome::TimedOut | Outcome::Rejected => {}
        }
    }
    let successes = latency_report(successes, elapsed, 1)?;
    let failures = latency_report(failures, elapsed, 1)?;
    let report = serde_json::json!({"successful": successes, "failed": failures, "errors": errors, "timing": timing});
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Outcome;
    use super::ReadSample;
    use super::latency_report;
    use super::read_report;

    #[test]
    fn read_failures_have_separate_counts_and_latency() -> anyhow::Result<()> {
        let sample = |latency_ns, status: Result<(), &str>| ReadSample {
            scheduled_ns: 0,
            invoked_ns: None,
            latency_ns,
            node: status.is_ok().then_some(0),
            retries: 0,
            phases: None,
            outcome: match status {
                Ok(()) => Outcome::Completed,
                Err(error) => Outcome::Failed(error.to_string()),
            },
        };
        let samples = vec![sample(3, Ok(())), sample(7, Err("expired")), sample(9, Err("expired"))];
        let actual = read_report(samples, Duration::from_secs(1))?;
        let expected = serde_json::json!({
            "successful": {"operations": 1, "operations_per_second": 1.0,
                "latency": {"samples": 1, "mean_ns": 3, "p50_ns": 3, "p95_ns": 3, "p99_ns": 3}},
            "failed": {"operations": 2, "operations_per_second": 2.0,
                "latency": {"samples": 2, "mean_ns": 8, "p50_ns": 7, "p95_ns": 9, "p99_ns": 9}},
            "errors": {"expired": 2},
            "timing": {"offered": 3, "admitted": 3, "completed": 1, "failed": 2,
                "timed_out": 0, "rejected": 0, "retries": 0, "phases": {
                    "samples": 0, "client_delay": null, "service": null, "queue_wait": null,
                    "generator_delay": null, "authorization": null,
                    "barrier_wait": null, "query_lock_wait": null, "query": null, "traces": []}},
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
}
