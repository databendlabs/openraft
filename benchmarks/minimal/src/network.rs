//! A minimized network implementation for benchmarking Openraft.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures::Stream;
use futures::StreamExt;
use futures::future;
use futures::stream;
use openraft::Config;
use openraft::OptionalSend;
use openraft::Raft;
use openraft::alias::SnapshotOf;
use openraft::alias::VoteOf;
use openraft::base::BoxFuture;
use openraft::base::BoxStream;
use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::error::Unreachable;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::stream_append_sequential;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendResult;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;

use crate::store::LogStore;
use crate::store::NodeId;
use crate::store::SnapshotData;
use crate::store::StateMachineStore;
use crate::store::TypeConfig;

pub type BenchRaft = Raft<TypeConfig, Arc<StateMachineStore>>;

/// Requests, or responses, that one direction of a pipelined `stream_append()` holds in flight.
///
/// It matches the responses that `Raft::stream_append()` keeps pending on the follower.
const PIPELINE_CAPACITY: usize = 64;

#[derive(Clone)]
pub struct Router {
    pub table: Arc<Mutex<BTreeMap<NodeId, BenchRaft>>>,
    append_delay: Duration,
    flush_delay: Duration,
    network_delay: Duration,
    sequential_append: bool,
    replication_read_delay: Duration,
    apply_read_delay: Duration,
    apply_io_delay: Duration,
    apply_entry_delay: Duration,
}

impl Router {
    pub fn new() -> Self {
        Router {
            table: Default::default(),
            append_delay: Duration::ZERO,
            flush_delay: Duration::ZERO,
            network_delay: Duration::ZERO,
            sequential_append: false,
            replication_read_delay: Duration::ZERO,
            apply_read_delay: Duration::ZERO,
            apply_io_delay: Duration::ZERO,
            apply_entry_delay: Duration::ZERO,
        }
    }

    /// Create a router with simulated log submission, flush, and one-way network delays.
    pub fn with_delays(append_delay: Duration, flush_delay: Duration, network_delay: Duration) -> Self {
        let mut router = Self::new();
        router.append_delay = append_delay;
        router.flush_delay = flush_delay;
        router.network_delay = network_delay;
        router
    }

    /// Replicate through `stream_append_sequential()`, one AppendEntries round trip at a time,
    /// instead of the pipelined `stream_append()`.
    pub fn with_sequential_append(mut self, sequential: bool) -> Self {
        self.sequential_append = sequential;
        self
    }

    /// Set the simulated delay of every log read the leader issues for replication.
    pub fn with_replication_read_delay(mut self, delay: Duration) -> Self {
        self.replication_read_delay = delay;
        self
    }

    /// Set the simulated delays of the state-machine apply path: the log read that loads each
    /// apply batch, the state-machine I/O of each `apply()` call, and the processing of each
    /// applied entry.
    pub fn with_apply_delays(mut self, read_delay: Duration, io_delay: Duration, entry_delay: Duration) -> Self {
        self.apply_read_delay = read_delay;
        self.apply_io_delay = io_delay;
        self.apply_entry_delay = entry_delay;
        self
    }

    pub fn get_raft(&self, id: NodeId) -> BenchRaft {
        self.table.lock().unwrap().get(&id).unwrap().clone()
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn new_cluster(&mut self, config: Arc<Config>, voter_ids: BTreeSet<NodeId>) -> anyhow::Result<()> {
        let mut rafts = BTreeMap::new();

        for id in voter_ids.iter() {
            let log_store = LogStore::with_delays(self.append_delay, self.flush_delay);
            let log_store = log_store.with_apply_read_delay(self.apply_read_delay);
            let log_store = log_store.with_replication_read_delay(self.replication_read_delay);
            let log_store = Arc::new(log_store);
            let sm = StateMachineStore::new().with_apply_io_delay(self.apply_io_delay);
            let sm = sm.with_apply_entry_delay(self.apply_entry_delay);
            let sm = Arc::new(sm);

            let raft = Raft::new(*id, config.clone(), self.clone(), log_store, sm).await?;

            rafts.insert(*id, raft);
        }

        {
            let mut t = self.table.lock().unwrap();
            *t = rafts.clone();
        }

        tracing::info!("--- initializing single node cluster: {}", 0);
        rafts.get_mut(&0).unwrap().initialize(voter_ids.clone()).await?;
        let log_index = 1; // log 0: initial membership log

        tracing::info!(log_index, "--- wait for init node to become leader");

        for (id, s) in rafts.iter_mut() {
            tracing::info!(log_index, "--- wait init log: {}, index: {}", id, log_index);
            s.wait(timeout()).applied_index(Some(log_index), "init").await?;
        }

        Ok(())
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl RaftNetworkFactory<TypeConfig> for Router {
    type Network = Network;

    async fn new_client(&mut self, target: NodeId, _node: &()) -> Self::Network {
        Network {
            target,
            target_raft: self.table.lock().unwrap().get(&target).unwrap().clone(),
            delay: self.network_delay,
            sequential_append: self.sequential_append,
        }
    }
}

pub struct Network {
    target: NodeId,
    target_raft: BenchRaft,
    /// One-way delay of every AppendEntries request and response.
    delay: Duration,
    sequential_append: bool,
}

impl RaftNetworkV2<TypeConfig> for Network {
    type SnapshotData = SnapshotData;

    /// Pass the request stream to the target's `Raft::stream_append()`, like a streaming
    /// transport.
    ///
    /// The leader reads the log for request k+1 while request k is on the wire or being
    /// appended, and the follower handles requests as they arrive. Each request and each
    /// response arrives one network delay after it was sent, without waiting for the others in
    /// flight.
    ///
    /// With `--sequential-append`, use `stream_append_sequential()`, which sends request k+1
    /// only after the response to request k arrives.
    fn stream_append<'s, S>(
        &'s mut self,
        input: S,
        option: RPCOption,
    ) -> BoxFuture<
        's,
        Result<BoxStream<'s, Result<AppendResult<TypeConfig>, RPCError<TypeConfig>>>, RPCError<TypeConfig>>,
    >
    where
        S: Stream<Item = AppendEntriesRequest<TypeConfig>> + OptionalSend + Unpin + 'static,
    {
        if self.sequential_append {
            return stream_append_sequential(self, input, option);
        }

        let requests = delayed(input, self.delay);
        let responses = delayed(self.target_raft.stream_append(requests), self.delay);
        let target = self.target;
        let responses =
            responses.map(move |result| result.map_err(|fatal| RPCError::Unreachable(unreachable(target, fatal))));
        let responses: BoxStream<'s, _> = Box::pin(responses);
        Box::pin(future::ready(Ok(responses)))
    }

    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendResult<TypeConfig>, RPCError<TypeConfig>> {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        let result = self.target_raft.append_entries(rpc).await;
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        result.map_err(|e| RPCError::Unreachable(self.unreachable(e)))
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<TypeConfig>,
        snapshot: SnapshotOf<TypeConfig, SnapshotData>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<TypeConfig>, StreamingError<TypeConfig>> {
        let resp = self.target_raft.install_full_snapshot(vote, snapshot).await.map_err(|e| self.unreachable(e))?;
        Ok(resp)
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        let resp = self.target_raft.vote(rpc).await.map_err(|e| self.unreachable(e))?;
        Ok(resp)
    }

    async fn transfer_leader(
        &mut self,
        req: TransferLeaderRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<TransferLeaderResponse<TypeConfig>, RPCError<TypeConfig>> {
        let resp = self.target_raft.handle_transfer_leader(req).await.map_err(|e| self.unreachable(e))?;
        Ok(resp)
    }
}

impl Network {
    /// The target returns an error only after a fatal error stopped it: report it as unreachable.
    fn unreachable<E>(&self, error: E) -> Unreachable<TypeConfig>
    where E: fmt::Display {
        unreachable(self.target, error)
    }
}

fn unreachable<E>(target: NodeId, error: E) -> Unreachable<TypeConfig>
where E: fmt::Display {
    Unreachable::from_string(format!("target={}: {}", target, error))
}

/// Deliver every item of `input` `delay` after `input` yields it, like one direction of a
/// network link with that latency.
///
/// A spawned task pulls `input` as soon as the link has room, up to [`PIPELINE_CAPACITY`] items
/// in flight, so an item does not wait for the delivery of the items before it.
fn delayed<T>(input: impl Stream<Item = T> + Send + 'static, delay: Duration) -> BoxStream<'static, T>
where T: Send + 'static {
    if delay.is_zero() {
        return Box::pin(input);
    }

    let (tx, rx) = tokio::sync::mpsc::channel(PIPELINE_CAPACITY);
    tokio::spawn(async move {
        let mut input = std::pin::pin!(input);
        while let Some(item) = input.next().await {
            // Wait for room before stamping, so a full link does not shorten the delay.
            let Ok(permit) = tx.reserve().await else {
                return;
            };
            permit.send((tokio::time::Instant::now() + delay, item));
        }
    });

    let output = stream::unfold(rx, |mut rx| async move {
        let (deliver_at, item) = rx.recv().await?;
        tokio::time::sleep_until(deliver_at).await;
        Some((item, rx))
    });
    Box::pin(output)
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delayed_items_in_flight_do_not_wait_for_each_other() {
        let delay = Duration::from_millis(50);

        let started = tokio::time::Instant::now();
        let items: Vec<_> = delayed(stream::iter([1, 2, 3]), delay).collect().await;
        let elapsed = started.elapsed();

        assert_eq!(vec![1, 2, 3], items);
        assert!(elapsed >= delay, "delivered after {elapsed:?}");
        assert!(elapsed < delay * 2, "items waited for each other: {elapsed:?}");
    }
}
