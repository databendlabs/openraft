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
use super::leader_change::Leaders;
use super::pace;
use super::request_sample::Outcome;
use super::request_sample::RequestSample;
use super::request_sample::Samples;
use super::request_sample::with_deadline;

#[derive(Clone)]
pub(super) struct WriteWorkload {
    pub(super) config: Arc<BenchConfig>,
    pub(super) leader: BenchRaft,
    pub(super) leaders: Option<Leaders>,
    pub(super) start: Arc<Barrier>,
    pub(super) epoch: Arc<OnceLock<Instant>>,
}

impl WriteWorkload {
    pub(super) fn spawn_clients(&self) -> Vec<Task> {
        if self.config.n_client == 0 {
            return Vec::new();
        }
        if !self.config.arrival.enabled() && self.leaders.is_none() {
            return self.spawn_sessions();
        }
        let workload = Workload {
            config: self.config.arrival.clone(),
            clients: self.config.n_client,
            operations: self.config.operations_per_client() * self.config.n_client / self.config.batch_size,
            rate: self.config.write_rate as f64 / self.config.batch_size as f64,
            measured: self.config.latency || self.config.read_clients > 0,
            stream_seed: super::arrival::WRITE_STREAM,
            start: self.start.clone(),
            epoch: self.epoch.clone(),
        };
        workload.spawn(self.clone())
    }

    fn spawn_sessions(&self) -> Vec<Task> {
        let n_client = self.config.n_client;
        let batch_size = self.config.batch_size;
        let ops_per_client = self.config.operations_per_client();
        let measure_latency = self.config.latency || self.config.read_clients > 0;
        let write_rate = self.config.write_rate;
        let mut handles = Vec::new();
        for client_id in 0..n_client {
            let l = self.leader.clone();
            let start = self.start.clone();
            let epoch = self.epoch.clone();
            let h = if batch_size <= 1 {
                // Single write mode
                tokio::spawn(async move {
                    let mut latencies = Vec::new();
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
                        l.client_write(ClientRequest {}).await?;
                        if let Some(started) = started {
                            let elapsed = started.elapsed();
                            let nanos = elapsed.as_nanos();
                            let nanos = u64::try_from(nanos)?;
                            latencies.push(nanos);
                        }
                    }
                    Ok::<_, anyhow::Error>(Samples::Writes(latencies))
                })
            } else {
                // Batch write mode
                tokio::spawn(async move {
                    let batches = ops_per_client / batch_size;
                    let mut latencies = Vec::new();
                    start.wait().await;
                    let epoch = *epoch.get().unwrap();

                    for batch in 0..batches {
                        let requests: Vec<_> = (0..batch_size).map(|_| ClientRequest {}).collect();
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
                            latencies.push(nanos);
                        }
                    }
                    Ok::<_, anyhow::Error>(Samples::Writes(latencies))
                })
            };

            handles.push(h)
        }

        handles
    }

    async fn sample(&self, arrival: Arrival) -> anyhow::Result<Option<RequestSample>> {
        let measure = self.config.latency || self.config.read_clients > 0;
        if !measure {
            self.write(&mut 0).await?;
            return Ok(None);
        }
        let epoch = *self.epoch.get().context("missing arrival epoch")?;
        let sampled = self.config.arrival.sampled(arrival.operation, super::arrival::WRITE_STREAM);
        let mut sample = RequestSample::new(epoch, arrival, sampled)?;
        let future = self.write(&mut sample.retries);
        let timeout = self.config.arrival.request_timeout_ms;
        let result = with_deadline(future, arrival.scheduled, timeout).await;
        match result {
            Ok(Some(())) => sample.node = Some(0),
            Ok(None) => sample.outcome = Outcome::TimedOut,
            Err(error) if self.config.arrival.open_loop => sample.outcome = Outcome::Failed(format!("{error:#}")),
            Err(error) => return Err(error),
        }
        sample.finish(epoch, arrival.scheduled)?;
        Ok(Some(sample))
    }

    async fn write(&self, retries: &mut u64) -> anyhow::Result<()> {
        if self.config.arrival.arrival_noop {
            return Ok(());
        }
        if let Some(leaders) = &self.leaders {
            return leaders.client_write(retries).await;
        }
        if self.config.batch_size == 1 {
            self.leader.client_write(ClientRequest {}).await?;
            return Ok(());
        }
        let requests: Vec<_> = (0..self.config.batch_size).map(|_| ClientRequest {}).collect();
        let mut stream = self.leader.client_write_many(requests, WriteOption::default()).await?;
        while let Some(result) = stream.try_next().await? {
            result?;
        }
        Ok(())
    }
}

impl Handler for WriteWorkload {
    async fn run(&self, arrival: Arrival) -> anyhow::Result<Option<RequestSample>> {
        self.sample(arrival).await
    }
}
