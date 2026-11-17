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
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendResult;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;

use crate::profile::Profile;
use crate::store::LogStore;
use crate::store::NodeId;
use crate::store::SnapshotData;
use crate::store::StateMachineStore;
use crate::store::TypeConfig;
use crate::wire_append::WireAppend;
use crate::wire_link::WireLink;

pub type BenchRaft = Raft<TypeConfig, Arc<StateMachineStore>>;

/// Requests, or responses, that one direction of a pipelined `stream_append()` holds in flight.
///
/// It matches the responses that `Raft::stream_append()` keeps pending on the follower.
const PIPELINE_CAPACITY: usize = 64;

#[derive(Clone)]
#[openraft::macros::since(version = "0.10.1", change = "optional application and directed-link profiles")]
pub struct Router {
    pub table: Arc<Mutex<BTreeMap<NodeId, BenchRaft>>>,
    append_delay: Duration,
    flush_delay: Duration,
    network_delay: Duration,
    replication_read_delay: Duration,
    apply_read_delay: Duration,
    apply_io_delay: Duration,
    apply_entry_delay: Duration,
    profile: Option<Arc<Profile>>,
    source: u64,
    links: Arc<Mutex<BTreeMap<(u64, u64), Arc<WireLink>>>>,
    stores: Arc<Mutex<BTreeMap<u64, Arc<LogStore>>>>,
    machines: Arc<Mutex<BTreeMap<u64, Arc<StateMachineStore>>>>,
}

impl Router {
    pub fn new() -> Self {
        Router {
            table: Default::default(),
            append_delay: Duration::ZERO,
            flush_delay: Duration::ZERO,
            network_delay: Duration::ZERO,
            replication_read_delay: Duration::ZERO,
            apply_read_delay: Duration::ZERO,
            apply_io_delay: Duration::ZERO,
            apply_entry_delay: Duration::ZERO,
            profile: None,
            source: 0,
            links: Arc::new(Mutex::new(BTreeMap::new())),
            stores: Arc::new(Mutex::new(BTreeMap::new())),
            machines: Arc::new(Mutex::new(BTreeMap::new())),
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

    /// Enable application/transport profiles while retaining the direct empty control.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn with_profile(mut self, profile: Profile) -> Self {
        if profile.profile {
            self.profile = Some(Arc::new(profile));
        }
        self
    }

    fn link(&self, source: u64, target: u64) -> Option<Arc<WireLink>> {
        let profile = self.profile.as_ref()?;
        if !profile.serialized_transport {
            return None;
        }
        let mut links = self.links.lock().unwrap();
        let key = (source, target);
        let link = links
            .entry(key)
            .or_insert_with(|| Arc::new(WireLink::new(profile.clone(), self.network_delay, source, target)));
        Some(link.clone())
    }

    /// Verify all profile state after drained work, and report bounded model observations.
    #[openraft::macros::since(version = "0.10.1")]
    pub async fn verify_profile(&self, drained: bool) -> anyhow::Result<serde_json::Value> {
        if !drained {
            let stores = self.stores.lock().unwrap();
            let machines = self.machines.lock().unwrap();
            let nodes: BTreeMap<_, _> = stores
                .iter()
                .map(|(id, store)| {
                    (
                        *id,
                        serde_json::json!({
                "log_reads":store.profile_report(),"transaction_delay":machines[id].profile_report()}),
                    )
                })
                .collect();
            return Ok(
                serde_json::json!({"verification":"pending_or_uncertain_writes","nodes":nodes,"links":self.link_report(),"profile":self.profile}),
            );
        }
        let rafts: Vec<_> = self.table.lock().unwrap().iter().map(|(id, raft)| (*id, raft.clone())).collect();
        let current = rafts[0]
            .1
            .current_leader()
            .await
            .ok_or_else(|| anyhow::anyhow!("no leader for final verification"))?;
        let leader = self.get_raft(current);
        let machine = leader.with_state_machine(|sm| Box::pin(async move { sm.clone() })).await?;
        let expected = machine.profile_state().await;
        let applied = expected.last_applied_log.map(|log| log.index());
        let mut nodes = BTreeMap::new();
        for (id, raft) in rafts {
            raft.wait(Some(Duration::from_secs(10))).applied_index_at_least(applied, "profile drain").await?;
            let machine = raft.with_state_machine(|sm| Box::pin(async move { sm.clone() })).await?;
            let actual = machine.profile_state().await;
            anyhow::ensure!(actual.values == expected.values, "profile values differ across nodes");
            let store = self.stores.lock().unwrap().get(&id).cloned().expect("node log store");
            nodes.insert(id, serde_json::json!({"keys":actual.values.len(),"applied":actual.last_applied_log,
                "duplicate_logical_ids":actual.duplicates,"dedup_evicted_ids":actual.evicted_ids,
                "dedup_retained_ids":actual.seen.len(),"log_reads":store.profile_report(),"transaction_delay":machine.profile_report()}));
        }
        Ok(
            serde_json::json!({"verification":"all_node_values_agree","nodes":nodes,"links":self.link_report(),
            "profile":self.profile,"duplicate_detection_scope":"bounded_recent_logical_id_window"}),
        )
    }

    /// Inspect a node's published application state without queueing behind apply work.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn profile_state(&self, node: u64) -> Option<crate::store::StateMachine> {
        let machines = self.machines.lock().unwrap();
        machines.get(&node)?.try_profile_state()
    }

    fn link_report(&self) -> BTreeMap<String, serde_json::Value> {
        self.links
            .lock()
            .unwrap()
            .iter()
            .map(|((source, target), link)| (format!("{source}->{target}"), link.report()))
            .collect()
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
            let log_store = match &self.profile {
                Some(profile) => log_store.with_profile(profile.clone(), *id),
                None => log_store,
            };
            let log_store = Arc::new(log_store);
            self.stores.lock().unwrap().insert(*id, log_store.clone());
            let sm = StateMachineStore::new().with_apply_io_delay(self.apply_io_delay);
            let sm = sm.with_apply_entry_delay(self.apply_entry_delay);
            let sm = match &self.profile {
                Some(profile) => sm.with_profile(profile.clone(), *id),
                None => sm,
            };
            let sm = Arc::new(sm);
            self.machines.lock().unwrap().insert(*id, sm.clone());
            let mut network = self.clone();
            network.source = *id;
            let raft = Raft::new(*id, config.clone(), network, log_store, sm).await?;

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
            request_link: self.link(self.source, target),
            response_link: self.link(target, self.source),
            capacity: self.profile.as_ref().map_or(PIPELINE_CAPACITY, |profile| profile.link_capacity),
        }
    }
}

#[openraft::macros::since(version = "0.10.1", change = "optional bounded serialized transport")]
pub struct Network {
    target: NodeId,
    target_raft: BenchRaft,
    /// One-way delay of every AppendEntries request and response.
    delay: Duration,
    request_link: Option<Arc<WireLink>>,
    response_link: Option<Arc<WireLink>>,
    capacity: usize,
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
    fn stream_append<'s, S>(
        &'s mut self,
        input: S,
        _option: RPCOption,
    ) -> BoxFuture<
        's,
        Result<BoxStream<'s, Result<AppendResult<TypeConfig>, RPCError<TypeConfig>>>, RPCError<TypeConfig>>,
    >
    where
        S: Stream<Item = AppendEntriesRequest<TypeConfig>> + OptionalSend + Unpin + 'static,
    {
        if let (Some(request), Some(response)) = (&self.request_link, &self.response_link) {
            let request = request.clone();
            let response = response.clone();
            let target = self.target;
            let capacity = self.capacity;
            let (requests_tx, requests_rx) = tokio::sync::mpsc::channel(capacity);
            let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
            let error = Arc::new(Mutex::new(None));
            let producer_error = error.clone();
            let requests = input.map(move |item| {
                let request = request.clone();
                async move { request.transmit(item).await }
            });
            tokio::spawn(async move {
                let mut requests = Box::pin(requests.buffered(capacity));
                loop {
                    let result = tokio::select! {
                        _ = &mut cancel_rx => return,
                        result = requests.next() => result,
                    };
                    let Some(result) = result else {
                        return;
                    };
                    match result {
                        Ok(item) => {
                            let sent = tokio::select! {
                                _ = &mut cancel_rx => return,
                                sent = requests_tx.send(item) => sent,
                            };
                            if sent.is_err() {
                                return;
                            }
                        }
                        Err(error) => {
                            *producer_error.lock().unwrap() = Some(error);
                            return;
                        }
                    }
                }
            });
            let requests = stream::unfold(requests_rx, |mut rx| async move {
                let item = rx.recv().await?;
                Some((item, rx))
            });
            let requests = Box::pin(requests);
            let responses = self.target_raft.stream_append(requests).map(move |result| {
                let response = response.clone();
                async move {
                    let reply = result.map_err(|error| RPCError::Unreachable(unreachable(target, error)))?;
                    let reply = response
                        .transmit(WireAppend::from(reply))
                        .await
                        .map_err(|error| RPCError::Unreachable(unreachable(target, error)))?;
                    Ok(reply.into())
                }
            });
            let tail = stream::once(async move {
                drop(cancel_tx);
                let error = error.lock().unwrap().take();
                error.map(|error| Err(RPCError::Unreachable(unreachable(target, error))))
            });
            let tail = tail.filter_map(future::ready);
            let responses = responses.buffered(capacity).chain(tail);
            let responses: BoxStream<'s, _> = Box::pin(responses);
            return Box::pin(future::ready(Ok(responses)));
        }
        let requests = delayed(input, self.delay);
        let responses = delayed(self.target_raft.stream_append(requests), self.delay);
        let target = self.target;
        let responses =
            responses.map(move |result| result.map_err(|fatal| RPCError::Unreachable(unreachable(target, fatal))));
        let responses: BoxStream<'s, _> = Box::pin(responses);
        Box::pin(future::ready(Ok(responses)))
    }

    /// Required by `RaftNetworkV2`, but Openraft calls it only from the default
    /// `stream_append()`, which [`Self::stream_append`] replaces, so replication never uses it.
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendResult<TypeConfig>, RPCError<TypeConfig>> {
        let rpc = self.send(rpc).await?;
        let result = self
            .target_raft
            .append_entries(rpc)
            .await
            .map_err(|error| RPCError::Unreachable(self.unreachable(error)))?;
        let reply = self.receive(WireAppend::from(result)).await?;
        Ok(reply.into())
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<TypeConfig>,
        snapshot: SnapshotOf<TypeConfig, SnapshotData>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<TypeConfig>, StreamingError<TypeConfig>> {
        let transfer = async {
            let request = (vote, snapshot.meta, snapshot.snapshot.into_inner());
            let (vote, meta, data) = self.send(request).await?;
            let snapshot = SnapshotOf::<TypeConfig, SnapshotData> {
                meta,
                snapshot: std::io::Cursor::new(data),
            };
            let resp = self.target_raft.install_full_snapshot(vote, snapshot).await.map_err(|e| self.unreachable(e))?;
            Ok(self.receive(resp).await?)
        };
        tokio::pin!(cancel);
        tokio::select! {
            closed = &mut cancel => Err(StreamingError::Closed(closed)),
            result = transfer => result,
        }
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        let rpc = self.send(rpc).await?;
        let resp = self.target_raft.vote(rpc).await.map_err(|e| self.unreachable(e))?;
        self.receive(resp).await
    }

    async fn pre_vote(
        &mut self,
        rpc: VoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        let rpc = self.send(rpc).await?;
        let resp = self.target_raft.pre_vote(rpc).await.map_err(|e| self.unreachable(e))?;
        self.receive(resp).await
    }

    async fn transfer_leader(
        &mut self,
        req: TransferLeaderRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<TransferLeaderResponse<TypeConfig>, RPCError<TypeConfig>> {
        let req = self.send(req).await?;
        let resp = self.target_raft.handle_transfer_leader(req).await.map_err(|e| self.unreachable(e))?;
        self.receive(resp).await
    }
}

impl Network {
    async fn send<T>(&self, value: T) -> Result<T, RPCError<TypeConfig>>
    where T: serde::Serialize + serde::de::DeserializeOwned {
        let Some(link) = &self.request_link else {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            return Ok(value);
        };
        link.transmit(value).await.map_err(|error| RPCError::Unreachable(self.unreachable(error)))
    }

    async fn receive<T>(&self, value: T) -> Result<T, RPCError<TypeConfig>>
    where T: serde::Serialize + serde::de::DeserializeOwned {
        let Some(link) = &self.response_link else {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            return Ok(value);
        };
        link.transmit(value).await.map_err(|error| RPCError::Unreachable(self.unreachable(error)))
    }

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

#[cfg(test)]
mod cancellation_tests {
    use std::pin::Pin;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::task::Context;
    use std::task::Poll;

    use super::*;

    struct Input {
        request: Option<AppendEntriesRequest<TypeConfig>>,
        dropped: Arc<AtomicBool>,
    }

    impl Stream for Input {
        type Item = AppendEntriesRequest<TypeConfig>;

        fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let Some(request) = self.request.take() else {
                return Poll::Pending;
            };
            Poll::Ready(Some(request))
        }
    }

    impl Drop for Input {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn dropping_output_cancels_a_producer_waiting_on_the_link() -> anyhow::Result<()> {
        let profile = Profile {
            profile: true,
            serialized_transport: true,
            ..Profile::default()
        };
        let mut router = Router::new().with_profile(profile);
        let config = Config {
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            ..Config::default()
        };
        router.new_cluster(Arc::new(config), BTreeSet::from([0])).await?;
        let mut network = router.new_client(0, &()).await;
        let profile = router.profile.as_ref().unwrap().clone();
        let link = Arc::new(WireLink::new(profile, Duration::from_secs(30), 0, 0));
        network.request_link = Some(link.clone());
        let dropped = Arc::new(AtomicBool::new(false));
        let request = AppendEntriesRequest {
            vote: openraft::impls::Vote::new_committed(1, 0),
            prev_log_id: None,
            entries: vec![],
            leader_commit: None,
        };
        let input = Input {
            request: Some(request),
            dropped: dropped.clone(),
        };
        let output = network.stream_append(input, RPCOption::new(Duration::from_secs(30))).await?;
        tokio::time::timeout(Duration::from_secs(1), async {
            while link.report()["messages"] == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        drop(output);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        let report = link.report();
        assert_eq!(0, report["current_encoded_buffer_bytes"]);
        router.get_raft(0).shutdown().await?;
        Ok(())
    }
}
