use std::sync::Arc;
use std::sync::OnceLock;
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
use tokio::sync::Barrier;

use super::BenchConfig;
use super::arrival::Arrival;
use super::arrival::Handler;
use super::arrival::Task;
use super::arrival::Workload;
use super::leader_change::Leaders;
use super::pace;
use super::request_sample::Completion;
use super::request_sample::LegacyReadSample;
use super::request_sample::Outcome;
use super::request_sample::RequestSample;
use super::request_sample::Samples;
use super::request_sample::nanos;
use super::request_sample::with_deadline;
use super::run_monitor::Live;

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
    pub(super) live: Arc<Live>,
}

impl ReadWorkload {
    pub(super) fn spawn_clients(&self) -> Vec<Task> {
        if !self.config.arrival.enabled()
            && self.leaders.is_none()
            && self.config.measurement.measurement_seconds.is_none()
        {
            return self.spawn_sessions();
        }
        let workload = Workload {
            config: self.config.arrival.clone(),
            clients: self.config.read_clients,
            operations: self.config.read_operations,
            rate: self.config.read_rate as f64,
            measured: true,
            stream_seed: super::arrival::READ_STREAM,
            measurement: self.config.measurement.clone(),
            trace_budget: self.config.trace_budget(),
            live: self.live.clone(),
            start: self.start.clone(),
            epoch: self.epoch.clone(),
        };
        workload.spawn(self.clone())
    }

    fn spawn_sessions(&self) -> Vec<Task> {
        let mut handles = Vec::new();
        for id in 0..self.config.read_clients {
            let workload = self.clone();
            let handle = tokio::spawn(async move {
                let samples = workload.run_session(id).await?;
                Ok(samples)
            });
            handles.push(handle);
        }
        handles
    }

    async fn run_session(&self, id: u64) -> anyhow::Result<Samples> {
        let operations = super::client_operations(self.config.read_operations, self.config.read_clients, id);
        let mut samples = Samples::new(self.config.measurement.bounds(), self.config.trace_budget());
        self.start.wait().await;
        let epoch = *self.epoch.get().context("missing arrival epoch")?;
        for sequence in 0..operations {
            let operation = sequence * self.config.read_clients + id;
            let sample = self.session_sample(epoch, operation).await?;
            samples.record_read(sample);
            let yield_due = (sequence + 1).is_multiple_of(64);
            if yield_due {
                tokio::task::yield_now().await;
            }
        }
        Ok(samples)
    }

    async fn session_sample(&self, epoch: Instant, operation: u64) -> anyhow::Result<LegacyReadSample> {
        let started = if self.config.read_rate > 0 {
            pace(epoch, operation, self.config.read_rate).await
        } else {
            Instant::now()
        };
        let result = self.session_read().await;
        let latency_ns = u64::try_from(started.elapsed().as_nanos())?;
        let status = match result {
            Ok(_) => Ok(()),
            Err(error) if self.config.allow_read_errors => Err(format!("{error:#}")),
            Err(error) => return Err(error),
        };
        Ok(LegacyReadSample { latency_ns, status })
    }

    async fn session_read(&self) -> anyhow::Result<u64> {
        let policy = ReadPolicy::from(self.config.read_policy);
        let option = LinearizerOption::from(policy).with_relaxed(self.config.read_relaxed);
        let linearizer = self.leader.get_local_read_linearizer(option).await?;
        if self.config.read_await_ready || self.config.read_query {
            let state = linearizer.await_ready(&self.reader).await?;
            let _ = std::hint::black_box(state);
        } else {
            let _ = std::hint::black_box(linearizer);
        }
        if self.config.read_query {
            let value = self.state_machine.read().await;
            let _ = std::hint::black_box(value);
        }
        let reader = self.config.read_on_follower.unwrap_or(0);
        Ok(reader)
    }

    async fn sample(&self, arrival: Arrival) -> anyhow::Result<ReadSample> {
        let epoch = *self.epoch.get().context("missing arrival epoch")?;
        let sampled = self.config.arrival.sampled(arrival.operation, super::arrival::READ_STREAM);
        let mut sample = RequestSample::new(epoch, arrival, sampled)?;
        let future = self.read(epoch, &mut sample);
        let timeout = self.config.arrival.request_timeout_ms;
        let deadline = self.config.measurement.bounds().deadline(epoch, arrival.scheduled, timeout);
        let result = with_deadline(future, deadline).await;
        self.read_outcome(&mut sample, result)?;
        sample.finish(epoch, arrival.scheduled)?;
        if sample.outcome == Outcome::Completed {
            if let Some(leaders) = &self.leaders {
                leaders.record_read(&sample);
            }
        }
        Ok(sample)
    }

    fn read_outcome(&self, sample: &mut ReadSample, result: anyhow::Result<Completion<u64>>) -> anyhow::Result<()> {
        match result {
            Ok(Completion::Completed(node)) => sample.node = Some(node),
            Ok(Completion::TimedOut) => sample.outcome = Outcome::TimedOut,
            Ok(Completion::Pending) => sample.outcome = Outcome::Pending,
            Err(error) if self.config.allow_read_errors || self.config.arrival.open_loop => {
                sample.outcome = Outcome::Failed(format!("{error:#}"));
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    /// Runs one read and returns the node that served it.
    async fn read(&self, epoch: Instant, sample: &mut ReadSample) -> anyhow::Result<u64> {
        sample.invoked = true;
        if self.leaders.is_some() {
            sample.invoked_ns = Some(nanos(epoch, Instant::now())?);
        }
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
            sample.mark_barrier(epoch)?;
        } else {
            let _ = std::hint::black_box(linearizer);
        }
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::measurement::Bounds;
    use super::Outcome;
    use super::ReadSample;
    use super::Samples;

    #[test]
    fn read_failures_have_separate_counts_and_latency() {
        let mut samples = Samples::new(
            Bounds {
                start: Duration::ZERO,
                end: None,
                deadline: None,
            },
            0,
        );
        for (latency_ns, outcome) in [
            (3, Outcome::Completed),
            (7, Outcome::Failed("expired".into())),
            (9, Outcome::Failed("expired".into())),
        ] {
            samples.record(ReadSample {
                scheduled_ns: 0,
                invoked_ns: None,
                invoked: false,
                latency_ns,
                node: None,
                retries: 0,
                phases: None,
                outcome,
            });
        }
        let actual = samples.read_report(Duration::from_secs(1));
        let expected_success = serde_json::json!({"operations":1,"operations_per_second":1.0,"completed_in_window":1,
            "operations_per_second_through_drain":1.0,"whole_workload_operations_per_second":1.0,
            "latency":{"samples":1,"mean_ns":3,"p50_ns":3,"p95_ns":3,"p99_ns":3}});
        let expected_failure = serde_json::json!({"operations":2,"operations_per_second":2.0,"completed_in_window":2,
            "latency":{"samples":2,"mean_ns":8,"p50_ns":7,"p95_ns":9,"p99_ns":9}});
        assert_eq!(expected_success, actual["successful"]);
        assert_eq!(expected_failure, actual["failed"]);
        let expected = serde_json::json!({"expired":2});
        assert_eq!(expected, actual["errors"]);
    }
}
