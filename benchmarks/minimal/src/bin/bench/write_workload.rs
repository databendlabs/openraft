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
use super::request_sample::Outcome;
use super::request_sample::RequestSample;
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

    async fn sample(&self, arrival: Arrival) -> anyhow::Result<Option<RequestSample>> {
        let measure = self.config.latency || self.config.read_clients > 0;
        if !measure {
            self.write(&mut 0).await?;
            return Ok(None);
        }
        let epoch = *self.epoch.get().context("missing arrival epoch")?;
        let every = self.config.arrival.phase_sample_every;
        let sampled = every > 0 && arrival.operation.is_multiple_of(every);
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
