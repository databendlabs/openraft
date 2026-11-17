use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Instant;

use anyhow::Context;
use bench_minimal::network::BenchRaft;
use bench_minimal::store::ClientRequest;
use futures::TryStreamExt;
use openraft::raft::WriteOption;
use tokio::sync::Barrier;

use super::BenchConfig;
use super::arrival::Arrival;
use super::arrival::Handler;
use super::arrival::Task;
use super::arrival::Workload;
use super::histogram::Histogram;
use super::leader_change::Leaders;
use super::pace;
use super::request_sample::Completion;
use super::request_sample::Outcome;
use super::request_sample::RequestSample;
use super::request_sample::Samples;
use super::request_sample::with_deadline;
use super::run_monitor::Live;
use super::write_progress::WriteProgress;

#[derive(Clone)]
pub(super) struct WriteWorkload {
    pub(super) config: Arc<BenchConfig>,
    pub(super) leader: BenchRaft,
    pub(super) leaders: Option<Leaders>,
    pub(super) start: Arc<Barrier>,
    pub(super) epoch: Arc<OnceLock<Instant>>,
    pub(super) live: Arc<Live>,
    pub(super) history: Option<Arc<super::profile_history::History>>,
}

impl WriteWorkload {
    pub(super) fn spawn_clients(&self) -> Vec<Task> {
        if self.config.n_client == 0 {
            return Vec::new();
        }
        if !self.config.arrival.enabled()
            && self.leaders.is_none()
            && self.config.measurement.measurement_seconds.is_none()
            && !self.config.profile.profile
        {
            return self.spawn_sessions();
        }
        let workload = Workload {
            config: self.config.arrival.clone(),
            clients: self.config.n_client,
            operations: self.config.n_operations / self.config.batch_size,
            entries_per_request: self.config.batch_size,
            rate: self.config.write_rate as f64 / self.config.batch_size as f64,
            measured: self.config.latency
                || self.config.profile.profile
                || self.config.read_clients > 0
                || self.config.measurement.measurement_seconds.is_some(),
            stream_seed: super::arrival::WRITE_STREAM,
            measurement: self.config.measurement.clone(),
            trace_budget: self.config.trace_budget(),
            live: self.live.clone(),
            start: self.start.clone(),
            epoch: self.epoch.clone(),
        };
        if self.config.profile.profile {
            return workload.spawn(ProfileWrites(self.clone()));
        }
        workload.spawn(self.clone())
    }

    fn spawn_sessions(&self) -> Vec<Task> {
        let n_client = self.config.n_client;
        let batch_size = self.config.batch_size;
        let measure_latency = self.config.latency || self.config.read_clients > 0;
        let write_rate = self.config.write_rate;
        let mut handles = Vec::new();
        for client_id in 0..n_client {
            let ops_per_client = self.config.operations_per_client(client_id);
            let bounds = self.config.measurement.bounds();
            let l = self.leader.clone();
            let start = self.start.clone();
            let epoch = self.epoch.clone();
            let h = if batch_size <= 1 {
                // Single write mode
                tokio::spawn(async move {
                    let mut latencies = Histogram::default();
                    start.wait().await;
                    let epoch = *epoch.get().unwrap();
                    for i in 0..ops_per_client {
                        let operation = i * n_client + client_id;
                        let started = if write_rate > 0 {
                            let scheduled = pace(epoch, operation, write_rate).await;
                            measure_latency.then_some(scheduled)
                        } else {
                            measure_latency.then(Instant::now)
                        };
                        l.client_write(ClientRequest::default()).await?;
                        if let Some(started) = started {
                            let elapsed = started.elapsed();
                            let nanos = elapsed.as_nanos();
                            let nanos = u64::try_from(nanos)?;
                            latencies.record(nanos);
                        }
                    }
                    let samples = Samples::from_completed(bounds, latencies, ops_per_client);
                    Ok::<_, anyhow::Error>(Box::new(samples))
                })
            } else {
                // Batch write mode
                tokio::spawn(async move {
                    let batches = ops_per_client / batch_size;
                    let mut latencies = Histogram::default();
                    start.wait().await;
                    let epoch = *epoch.get().unwrap();

                    for batch in 0..batches {
                        let requests: Vec<_> = (0..batch_size).map(|_| ClientRequest::default()).collect();
                        let operation = (batch * n_client + client_id) * batch_size;
                        let started = if write_rate > 0 {
                            let scheduled = pace(epoch, operation, write_rate).await;
                            measure_latency.then_some(scheduled)
                        } else {
                            measure_latency.then(Instant::now)
                        };
                        let mut stream = l.client_write_many(requests, WriteOption::default()).await?;
                        while let Some(result) = stream.try_next().await? {
                            result?;
                        }
                        if let Some(started) = started {
                            let elapsed = started.elapsed();
                            let nanos = elapsed.as_nanos();
                            let nanos = u64::try_from(nanos)?;
                            latencies.record(nanos);
                        }
                    }
                    let samples = Samples::from_completed(bounds, latencies, ops_per_client);
                    let samples = samples.with_batch_size(batch_size);
                    Ok::<_, anyhow::Error>(Box::new(samples))
                })
            };

            handles.push(h)
        }

        handles
    }

    async fn sample(&self, arrival: Arrival) -> anyhow::Result<Option<RequestSample>> {
        let measure = self.config.latency
            || self.config.read_clients > 0
            || self.config.measurement.measurement_seconds.is_some();
        if !measure {
            self.write(&mut 0, None).await?;
            return Ok(None);
        }
        let epoch = *self.epoch.get().context("missing arrival epoch")?;
        let sampled = self.config.arrival.sampled(arrival.operation, super::arrival::WRITE_STREAM);
        let mut sample = RequestSample::new(epoch, arrival, sampled)?;
        if self.config.batch_size > 1 && !self.config.arrival.arrival_noop {
            let progress = WriteProgress::new(epoch, self.config.measurement.bounds(), self.config.batch_size);
            sample.write_progress = Some(Box::new(progress));
        }
        let future = async {
            sample.invoked = true;
            self.write(&mut sample.retries, sample.write_progress.as_deref_mut()).await
        };
        let timeout = self.config.arrival.request_timeout_ms;
        let deadline = self.config.measurement.bounds().deadline(epoch, arrival.scheduled, timeout);
        let result = with_deadline(future, deadline).await;
        match result {
            Ok(Completion::Completed(())) => sample.node = Some(0),
            Ok(Completion::TimedOut) => sample.outcome = Outcome::TimedOut,
            Ok(Completion::Pending) => sample.outcome = Outcome::Pending,
            Err(error) if self.config.arrival.open_loop => sample.outcome = Outcome::Failed(format!("{error:#}")),
            Err(error) => return Err(error),
        }
        sample.finish(epoch, arrival.scheduled)?;
        Ok(Some(sample))
    }

    async fn write(&self, retries: &mut u64, mut progress: Option<&mut WriteProgress>) -> anyhow::Result<()> {
        if self.config.arrival.arrival_noop {
            return Ok(());
        }
        if let Some(leaders) = &self.leaders {
            leaders.write_request(ClientRequest::default(), retries).await?;
            return Ok(());
        }
        if self.config.batch_size == 1 {
            self.leader.client_write(ClientRequest::default()).await?;
            return Ok(());
        }
        let requests: Vec<_> = (0..self.config.batch_size).map(|_| ClientRequest::default()).collect();
        let mut stream = self.leader.client_write_many(requests, WriteOption::default()).await?;
        while let Some(result) = stream.try_next().await? {
            if let Some(progress) = &mut progress {
                progress.acknowledge(result.is_ok());
            }
            result?;
        }
        Ok(())
    }

    async fn profile_sample(&self, arrival: Arrival) -> anyhow::Result<Option<RequestSample>> {
        let measure = self.config.latency
            || self.config.profile.profile
            || self.config.read_clients > 0
            || self.config.measurement.measurement_seconds.is_some();
        if !measure {
            self.profile_write(arrival.operation, &mut 0, None).await?;
            return Ok(None);
        }
        let epoch = *self.epoch.get().context("missing arrival epoch")?;
        let sampled = self.config.arrival.sampled(arrival.operation, super::arrival::WRITE_STREAM);
        let mut sample = RequestSample::new(epoch, arrival, sampled)?;
        if self.config.batch_size > 1 && !self.config.arrival.arrival_noop {
            let progress = WriteProgress::new(epoch, self.config.measurement.bounds(), self.config.batch_size);
            sample.write_progress = Some(Box::new(progress));
        }
        let mut invoked = None;
        let future = async {
            sample.invoked = true;
            if self.history.is_some() {
                invoked = Some(Instant::now());
            }
            self.profile_write(
                arrival.operation,
                &mut sample.retries,
                sample.write_progress.as_deref_mut(),
            )
            .await
        };
        let timeout = self.config.arrival.request_timeout_ms;
        let deadline = self.config.measurement.bounds().deadline(epoch, arrival.scheduled, timeout);
        let result = with_deadline(future, deadline).await;
        match result {
            Ok(Completion::Completed(())) => sample.node = Some(0),
            Ok(Completion::TimedOut) => sample.outcome = Outcome::TimedOut,
            Ok(Completion::Pending) => sample.outcome = Outcome::Pending,
            Err(error) if self.config.arrival.open_loop => sample.outcome = Outcome::Failed(format!("{error:#}")),
            Err(error) => return Err(error),
        }
        sample.finish(epoch, arrival.scheduled)?;
        if let Some(history) = &self.history {
            history.write_outcome(arrival, &sample, invoked);
        }
        Ok(Some(sample))
    }

    async fn profile_write(
        &self,
        operation: u64,
        retries: &mut u64,
        mut progress: Option<&mut WriteProgress>,
    ) -> anyhow::Result<()> {
        if self.config.arrival.arrival_noop {
            return Ok(());
        }
        let first = operation * self.config.batch_size;
        let invoked = if self.history.is_some() {
            let now = Instant::now();
            Some(now)
        } else {
            None
        };
        if let Some(leaders) = &self.leaders {
            let request = self.config.profile.request(first);
            let response = leaders.write_request(request, retries).await?;
            if let Some(history) = &self.history {
                let invoked = invoked.expect("profile invocation clock");
                history.acknowledged(first, response.log_id, invoked);
            }
            return Ok(());
        }
        if self.config.batch_size == 1 {
            let request = self.config.profile.request(first);
            let response = self.leader.client_write(request).await?;
            if let Some(history) = &self.history {
                let invoked = invoked.expect("profile invocation clock");
                history.acknowledged(first, response.log_id, invoked);
            }
            return Ok(());
        }
        let requests: Vec<_> =
            (0..self.config.batch_size).map(|offset| self.config.profile.request(first + offset)).collect();
        let mut offset = 0;
        let mut stream = self.leader.client_write_many(requests, WriteOption::default()).await?;
        while let Some(result) = stream.try_next().await? {
            if let Some(progress) = &mut progress {
                progress.acknowledge(result.is_ok());
            }
            let response = result?;
            if let Some(history) = &self.history {
                let invoked = invoked.expect("profile invocation clock");
                history.acknowledged(first + offset, response.log_id, invoked);
            }
            offset += 1;
        }
        Ok(())
    }
}

impl Handler for WriteWorkload {
    async fn run(&self, arrival: Arrival) -> anyhow::Result<Option<RequestSample>> {
        self.sample(arrival).await
    }
}

/// Select profile bookkeeping once, so the empty handler keeps its original hot path.
#[derive(Clone)]
struct ProfileWrites(WriteWorkload);

impl Handler for ProfileWrites {
    async fn run(&self, arrival: Arrival) -> anyhow::Result<Option<RequestSample>> {
        self.0.profile_sample(arrival).await
    }
}
