//! Fixtures for testing Raft.

#![allow(clippy::uninlined_format_args)]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Context;
use openraft::Config;
use openraft::RPCTypes;
use openraft::Raft;
use openraft::RaftMetrics;
use openraft::RaftState;
use openraft::RaftTypeConfig;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::async_runtime::Mutex as AsyncMutex;
use openraft::errors::ClientWriteError;
use openraft::errors::Fatal;
use openraft::errors::Infallible;
use openraft::errors::LinearizableReadError;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft::errors::RaftError;
use openraft::metrics::Wait;
use openraft::metrics::WaitError;
use openraft::raft::ClientWriteResponse;
use openraft::raft::linearizable_read::ReadLogId;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::MutexOf;
use openraft::vote::RaftLeaderId;
use openraft::vote::RaftLeaderIdExt;
use openraft_legacy::network_v1::InstallSnapshotRequest;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::MemLogStore as LogStoreInner;
use openraft_memstore::MemNodeId;
use openraft_memstore::MemStateMachine as SMInner;
use openraft_memstore::TypeConfig;
use openraft_memstore::TypeConfig as MemConfig;

use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::rpc_response::RpcResponse;
use crate::fixtures::rpc_rule::RpcRule;

mod chunked_network;
pub mod custom_membership;
pub mod events;
mod harness;
pub use harness::ut_harness;
pub mod logging;
mod network;
mod network_delay;
pub use network_delay::delay_from_env;
mod node;
pub use node::Node;
mod post_hook;
mod pre_hook;
pub mod rpc_error_type;
pub mod rpc_kind;
pub mod rpc_request;
pub mod rpc_response;
pub mod rpc_rule;
mod write_summary;
pub use write_summary::WriteSummary;

pub type MemSnapshotData = Cursor<Vec<u8>>;
pub type MemRpcRequest = RpcRequest<TypeConfig, MemSnapshotData>;
pub type MemRpcResponse = RpcResponse<TypeConfig>;

pub type MemLogStore = Arc<LogStoreInner>;
pub type MemStateMachine = Arc<SMInner>;

/// A concrete Raft type used during testing.
pub type MemRaft = Raft<MemConfig, MemStateMachine>;

pub const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn log_id(term: u64, node_id: u64, index: u64) -> LogIdOf<TypeConfig> {
    LogIdOf::<TypeConfig>::new(
        <TypeConfig as RaftTypeConfig>::LeaderId::new_committed(term, node_id),
        index,
    )
}

#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
#[derive(Hash)]
pub enum Direction {
    NetSend,
    NetRecv,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetSend => write!(f, "sending from"),
            NetRecv => write!(f, "receiving by"),
        }
    }
}

use Direction::NetRecv;
use Direction::NetSend;
use openraft::alias::LogIdOf;
use openraft::alias::VoteOf;
use openraft::async_runtime::WatchReceiver;
use post_hook::PostHook;
use post_hook::PostHookResult;
pub use post_hook::RpcResponseCall;
use pre_hook::PreHook;
use pre_hook::PreHookResult;
pub use pre_hook::RpcCall;
use rpc_error_type::RpcErrorType;

/// A type which emulates a network transport and implements the `RaftNetworkFactory` trait.
#[derive(Clone)]
pub struct RaftRouter {
    /// The Raft runtime config which all nodes are using.
    config: Arc<Config>,

    /// The table of all nodes currently known to this router instance.
    nodes: Arc<Mutex<BTreeMap<MemNodeId, Node>>>,

    /// Whether to save the committed entries to the RaftLogStorage.
    pub enable_saving_committed: bool,

    /// Whether to fail a network RPC that is sent from/to a node.
    /// And it defines what kind of error to return.
    fail_rpc: Arc<Mutex<HashMap<(MemNodeId, Direction), RpcErrorType>>>,

    /// To emulate network delay for sending, in milliseconds.
    /// 0 means no delay.
    send_delay: Arc<AtomicU64>,

    /// To simulate PartialSuccess for AppendEntries RPCs.
    ///
    /// If the quota is set to `Some(n)`, then the AppendEntries RPC consumes the quota,
    /// and send out at most `n` entries.
    append_entries_quota: Arc<Mutex<Option<u64>>>,

    /// Count of RPCs sent.
    rpc_count: Arc<Mutex<HashMap<RpcKind, u64>>>,
    rpc_rules: Arc<Mutex<BTreeMap<u64, RpcRule>>>,
    next_rpc_rule: Arc<AtomicU64>,

    /// A hook function to be called when before an RPC is sent to target node.
    rpc_pre_hook: Arc<MutexOf<TypeConfig, HashMap<RpcKind, PreHook>>>,

    /// A hook function to be called when after an RPC is received from target node.
    rpc_post_hook: Arc<MutexOf<TypeConfig, HashMap<RpcKind, PostHook>>>,

    /// Recorded wire chunks when the legacy snapshot transport is selected.
    snapshot_chunks: Option<Arc<Mutex<Vec<InstallSnapshotRequest<MemConfig>>>>>,
}

pub struct Builder {
    config: Arc<Config>,
    send_delay: u64,
    chunked_snapshots: bool,
}

impl Builder {
    /// Exercise the legacy chunking adapter when replicating snapshots.
    pub fn chunked_snapshots(mut self) -> Self {
        self.chunked_snapshots = true;
        self
    }

    pub fn send_delay(mut self, ms: u64) -> Self {
        self.send_delay = ms;
        self
    }

    pub fn build(self) -> RaftRouter {
        let send_delay = network_delay::configured_network_send_delay(self.send_delay);
        let snapshot_chunks = if self.chunked_snapshots {
            Some(Arc::new(Mutex::new(Vec::new())))
        } else {
            None
        };
        tracing::info!(send_delay_ms = send_delay, "router network delay");
        let send_delay = Arc::new(AtomicU64::new(send_delay));
        harness::record_network_delay(&send_delay);
        RaftRouter {
            config: self.config,
            nodes: Default::default(),
            enable_saving_committed: true,
            fail_rpc: Default::default(),
            send_delay,
            append_entries_quota: Arc::new(Mutex::new(None)),
            rpc_count: Default::default(),
            rpc_rules: Default::default(),
            next_rpc_rule: Default::default(),
            rpc_pre_hook: Arc::new(TypeConfig::mutex(HashMap::new())),
            rpc_post_hook: Arc::new(TypeConfig::mutex(HashMap::new())),
            snapshot_chunks,
        }
    }
}

impl RaftRouter {
    /// Validate the visible scenario config and prepare a router builder.
    pub fn builder(config: Config) -> anyhow::Result<Builder> {
        let config = config.validate()?;
        Ok(Builder {
            config: Arc::new(config),
            send_delay: 0,
            chunked_snapshots: false,
        })
    }

    /// Create a router, validating the scenario config before starting any nodes.
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let builder = Self::builder(config)?;
        Ok(builder.build())
    }

    pub fn network_send_delay(&mut self, ms: u64) {
        tracing::info!(send_delay_ms = ms, "router network delay changed");
        self.send_delay.store(ms, Ordering::Relaxed);
    }

    /// Read the complete chunk sequence sent through the selected legacy transport.
    pub fn snapshot_chunks(&self) -> Vec<InstallSnapshotRequest<MemConfig>> {
        let chunks = self.snapshot_chunks.as_ref().expect("select the chunked snapshot transport");
        let chunks = chunks.lock().unwrap();
        chunks.clone()
    }

    async fn rand_send_delay(&self) {
        let send_delay = self.send_delay.load(Ordering::Relaxed);
        network_delay::apply_send_delay::<TypeConfig>(send_delay).await;
    }

    pub fn set_append_entries_quota(&mut self, quota: Option<u64>) {
        let mut append_entries_quota = self.append_entries_quota.lock().unwrap();
        *append_entries_quota = quota;
    }

    fn count_rpc(&self, kind: RpcKind) {
        let mut counts = self.rpc_count.lock().unwrap();
        let count = counts.entry(kind).or_insert(0);
        *count += 1;
    }

    /// Read attempt counts grouped by the production API's RPC types.
    pub fn get_rpc_count(&self) -> HashMap<RPCTypes, u64> {
        let counts = self.rpc_count.lock().unwrap();
        let mut grouped = HashMap::new();
        for (kind, count) in counts.iter() {
            let rpc_type = kind.rpc_type();
            let total = grouped.entry(rpc_type).or_insert(0);
            *total += count;
        }
        grouped
    }

    /// Read counts that distinguish Vote from PreVote and full snapshots from chunks.
    pub fn get_rpc_kind_count(&self) -> HashMap<RpcKind, u64> {
        let counts = self.rpc_count.lock().unwrap();
        counts.clone()
    }

    /// Add a fault rule after all existing rules and return its removal ID.
    pub fn add_rpc_rule(&self, rule: RpcRule) -> u64 {
        let id = self.next_rpc_rule.fetch_add(1, Ordering::Relaxed);
        let mut rules = self.rpc_rules.lock().unwrap();
        rules.insert(id, rule);
        id
    }

    /// Remove exactly one fault rule while retaining all other rules.
    pub fn remove_rpc_rule(&self, id: u64) {
        let mut rules = self.rpc_rules.lock().unwrap();
        let removed = rules.remove(&id);
        removed.expect("RPC rule is not installed");
    }

    /// Create a cluster: 0 is the initial leader, others are voters and learners
    ///
    /// NOTE: it create a single node cluster first, then change it to a multi-voter cluster.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn new_cluster<V, L>(&mut self, voter_ids: V, learners: L) -> anyhow::Result<u64>
    where
        V: IntoIterator<Item = MemNodeId>,
        L: IntoIterator<Item = MemNodeId>,
    {
        let voter_ids = voter_ids.into_iter().collect::<BTreeSet<_>>();
        let learners = learners.into_iter().collect::<BTreeSet<_>>();
        self.validate_cluster(&voter_ids, &learners)?;
        let leader_id = MemNodeId::default();

        self.new_raft_node(leader_id).await?;

        tracing::info!("--- wait for init node to ready");

        self.wait(&leader_id).applied_index(None, "empty").await?;

        tracing::info!("--- initializing single node cluster: {}", 0);

        self.initialize(leader_id).await?;
        let mut log_index = 1; // log 0: initial membership log; log 1: leader initial log

        tracing::info!(log_index, "--- wait for init node to become leader");

        self.wait(&leader_id).applied_index(Some(log_index), "init").await?;
        self.wait(&leader_id).vote(VoteOf::<MemConfig>::new_committed(1, 0), "init vote").await?;

        for id in voter_ids.iter() {
            if *id == leader_id {
                continue;
            }
            tracing::info!(log_index, "--- add voter: {}", id);

            self.new_raft_node(*id).await?;
            let response = self.add_learner(leader_id, *id).await?;
            log_index = response.log_id.index();

            self.wait(id).state(ServerState::Learner, "empty node").await?;
        }

        for id in voter_ids.iter() {
            self.wait(id).applied_index(Some(log_index), &format!("learners of {:?}", voter_ids)).await?;
        }

        if voter_ids.len() > 1 {
            tracing::info!(log_index, "--- change membership to setup voters: {:?}", voter_ids);

            let node = self.get_raft_handle(&MemNodeId::default())?;
            let response = node.change_membership(voter_ids.clone(), false).await?;
            log_index = response.log_id.index();

            for id in voter_ids.iter() {
                self.wait(id).applied_index(Some(log_index), &format!("cluster of {:?}", voter_ids)).await?;
            }
        }

        for &id in &learners {
            tracing::info!(log_index, "--- add learner: {}", id);
            self.new_raft_node(id).await?;
            let response = self.add_learner(leader_id, id).await?;
            log_index = response.log_id.index();
        }
        for id in learners.iter() {
            self.wait(id).applied_index(Some(log_index), &format!("learners of {:?}", learners)).await?;
        }

        Ok(log_index)
    }

    fn validate_cluster(&self, voters: &BTreeSet<MemNodeId>, learners: &BTreeSet<MemNodeId>) -> anyhow::Result<()> {
        anyhow::ensure!(voters.contains(&0), "cluster voters must include node 0");
        let disjoint = voters.is_disjoint(learners);
        anyhow::ensure!(disjoint, "cluster voters and learners must be disjoint");
        let nodes = self.nodes.lock().unwrap();
        for id in voters.union(learners) {
            let registered = nodes.contains_key(id);
            anyhow::ensure!(!registered, "node {id} is already registered");
        }
        Ok(())
    }

    /// Create and register a new Raft node bearing the given ID.
    pub async fn new_raft_node(&mut self, id: MemNodeId) -> anyhow::Result<()> {
        let (log_store, sm) = self.new_store();
        self.new_raft_node_with_sto(id, log_store, sm).await
    }

    pub fn new_store(&mut self) -> (MemLogStore, MemStateMachine) {
        let (log, sm) = openraft_memstore::new_mem_store();
        log.enable_saving_committed.store(self.enable_saving_committed, Ordering::Relaxed);
        (log, sm)
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn new_raft_node_with_sto(
        &mut self,
        id: MemNodeId,
        log_store: MemLogStore,
        sm: MemStateMachine,
    ) -> anyhow::Result<()> {
        {
            let nodes = self.nodes.lock().unwrap();
            anyhow::ensure!(!nodes.contains_key(&id), "node {id} is already registered");
        }
        let node = Raft::new(id, self.config.clone(), self.clone(), log_store.clone(), sm.clone()).await?;
        let mut rt = self.nodes.lock().unwrap();
        rt.insert(id, Node {
            raft: node,
            log_store,
            state_machine: sm,
        });
        Ok(())
    }

    /// Detach a live node from routing while preserving its network faults.
    pub fn detach_node(&mut self, id: MemNodeId) -> Option<Node> {
        let mut nodes = self.nodes.lock().unwrap();
        nodes.remove(&id)
    }

    /// Stop a node and retain its handle and stores without healing its network.
    pub async fn stop_node(&mut self, id: MemNodeId) -> anyhow::Result<Node> {
        let detached = self.detach_node(id);
        let node = detached.with_context(|| format!("node {id} is not registered"))?;
        node.raft.shutdown().await?;
        Ok(node)
    }

    /// Initialize cluster with the config that contains all nodes.
    pub async fn initialize(&self, node_id: MemNodeId) -> anyhow::Result<()> {
        let members: BTreeSet<MemNodeId> = {
            let rt = self.nodes.lock().unwrap();
            rt.keys().cloned().collect()
        };

        tracing::info!(
            node_id = display(node_id),
            members = debug(&members),
            "initializing cluster"
        );

        let node = self.get_raft_handle(&node_id)?;
        node.initialize(members.clone()).await?;
        Ok(())
    }

    /// Isolate the network of the specified node.
    #[tracing::instrument(level = "debug", skip(self))]
    pub fn set_network_error(&self, id: MemNodeId, emit_failure: bool) {
        let v = if emit_failure {
            Some(RpcErrorType::NetworkError)
        } else {
            None
        };

        self.set_rpc_failure(id, NetRecv, v);
        self.set_rpc_failure(id, NetSend, v);
    }

    /// Set to `true` to return [`Unreachable`](`openraft::errors::Unreachable`) when sending RPC to
    /// a node.
    pub fn set_unreachable(&self, id: MemNodeId, unreachable: bool) {
        let v = if unreachable {
            Some(RpcErrorType::Unreachable)
        } else {
            None
        };
        self.set_rpc_failure(id, NetRecv, v);
        self.set_rpc_failure(id, NetSend, v);
    }

    /// Set whether to emit a specified rpc error when sending to/receiving from a node.
    pub fn set_rpc_failure(&self, id: MemNodeId, dir: Direction, rpc_error_type: Option<RpcErrorType>) {
        let mut fails = self.fail_rpc.lock().unwrap();
        if let Some(rpc_error_type) = rpc_error_type {
            fails.insert((id, dir), rpc_error_type);
        } else {
            fails.remove(&(id, dir));
        }
    }

    /// Register a pre-hook for one exact [`RpcKind`].
    ///
    /// `hook` returns an ordinary future. This method boxes that future.
    pub async fn set_rpc_pre_hook<F, Fut>(&self, kind: RpcKind, hook: F)
    where
        F: Fn(RpcCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), RPCError<TypeConfig, Infallible>>> + Send + 'static,
    {
        let stored = move |call| -> PreHookResult {
            let future = hook(call);
            Box::pin(future)
        };
        let stored: PreHook = Box::new(stored);
        self.rpc_pre_hook(kind, Some(stored)).await;
    }

    /// Register a post-hook for one exact [`RpcKind`].
    ///
    /// The hook sees the final transport response and returns an ordinary future.
    /// This method boxes that future.
    pub async fn set_rpc_post_hook<F, Fut>(&self, kind: RpcKind, hook: F)
    where
        F: Fn(RpcResponseCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), RPCError<TypeConfig>>> + Send + 'static,
    {
        let stored = move |call| -> PostHookResult {
            let future = hook(call);
            Box::pin(future)
        };
        let stored: PostHook = Box::new(stored);
        self.rpc_post_hook(kind, Some(stored)).await;
    }

    /// Remove the pre-hook for one [`RpcKind`].
    pub async fn clear_rpc_pre_hook(&self, kind: RpcKind) {
        self.rpc_pre_hook(kind, None).await;
    }

    /// Remove the post-hook for one [`RpcKind`].
    pub async fn clear_rpc_post_hook(&self, kind: RpcKind) {
        self.rpc_post_hook(kind, None).await;
    }

    async fn rpc_pre_hook(&self, kind: RpcKind, hook: Option<PreHook>) {
        let mut rpc_pre_hook = self.rpc_pre_hook.lock().await;
        if let Some(hook) = hook {
            rpc_pre_hook.insert(kind, hook);
        } else {
            rpc_pre_hook.remove(&kind);
        }
    }

    async fn rpc_post_hook(&self, kind: RpcKind, hook: Option<PostHook>) {
        let mut post_hook = self.rpc_post_hook.lock().await;
        if let Some(hook) = hook {
            post_hook.insert(kind, hook);
        } else {
            post_hook.remove(&kind);
        }
    }

    async fn before_rpc<E>(
        &self,
        request: impl Into<MemRpcRequest>,
        from: MemNodeId,
        to: MemNodeId,
    ) -> Result<(), RPCError<MemConfig, E>>
    where
        E: std::error::Error,
    {
        let request = request.into();
        let kind = request.kind();
        self.count_rpc(kind);
        self.call_rpc_pre_hook(request, from, to).await?;
        self.emit_rpc_error(from, to)?;
        self.rand_send_delay().await;
        Ok(())
    }

    /// Call pre-hook before an RPC is sent.
    #[allow(clippy::result_large_err)]
    async fn call_rpc_pre_hook<E>(
        &self,
        request: impl Into<MemRpcRequest>,
        from: MemNodeId,
        to: MemNodeId,
    ) -> Result<(), RPCError<MemConfig, E>>
    where
        E: std::error::Error,
    {
        let request = request.into();
        let kind = request.kind();

        {
            let rules = self.rpc_rules.lock().unwrap();
            for (id, rule) in rules.iter() {
                if rule.matches(&request, from, to) {
                    tracing::debug!(id, ?rule, "RPC blocked by fault rule");
                    let error = rule.error.make_error(from, NetSend);
                    return Err(error);
                }
            }
        }

        let fu = {
            let rpc_pre_hook = self.rpc_pre_hook.lock().await;

            let Some(hook) = rpc_pre_hook.get(&kind) else {
                return Ok(());
            };

            let router = self.clone();
            let call = RpcCall {
                router,
                request,
                from,
                to,
            };
            hook(call)
        };

        let res = fu.await;

        match res {
            Ok(()) => Ok(()),
            Err(err) => {
                // The pre-hook should only return RPCError variants
                let rpc_err = match err {
                    RPCError::Timeout(e) => e.into(),
                    RPCError::Unreachable(e) => e.into(),
                    RPCError::Network(e) => e.into(),
                    RPCError::RemoteError(e) => {
                        unreachable!("unexpected RemoteError: {:?}", e);
                    }
                };
                Err(rpc_err)
            }
        }
    }

    /// Call the post-hook after the final transport response is formed.
    #[allow(clippy::result_large_err)]
    async fn call_rpc_post_hook<E>(
        &self,
        request: impl Into<MemRpcRequest>,
        response: impl Into<MemRpcResponse>,
        from: MemNodeId,
        to: MemNodeId,
    ) -> Result<(), RPCError<MemConfig, E>>
    where
        E: std::error::Error,
    {
        let request = request.into();
        let response = response.into();
        let kind = request.kind();

        let fu = {
            let rpc_post_hook = self.rpc_post_hook.lock().await;

            let Some(hook) = rpc_post_hook.get(&kind) else {
                return Ok(());
            };

            let router = self.clone();
            let call = RpcResponseCall {
                router,
                request,
                response,
                from,
                to,
            };
            hook(call)
        };

        let res = fu.await;

        match res {
            Ok(()) => Ok(()),
            Err(err) => {
                // The pre-hook should only return RPCError variants
                let rpc_err = match err {
                    RPCError::Timeout(e) => e.into(),
                    RPCError::Unreachable(e) => e.into(),
                    RPCError::Network(e) => e.into(),
                    RPCError::RemoteError(e) => {
                        unreachable!("unexpected RemoteError: {:?}", e);
                    }
                };
                Err(rpc_err)
            }
        }
    }

    /// Get a payload of the latest metrics from each node in the cluster.
    #[allow(clippy::significant_drop_in_scrutinee)]
    pub fn latest_metrics(&self) -> Vec<RaftMetrics<MemConfig>> {
        let rt = self.nodes.lock().unwrap();
        let mut metrics = vec![];
        for node in rt.values() {
            let m = node.raft.metrics().borrow_watched().clone();
            tracing::debug!("router::latest_metrics: {:?}", m);
            metrics.push(m);
        }
        metrics
    }

    pub fn get_metrics(&self, node_id: &MemNodeId) -> anyhow::Result<RaftMetrics<MemConfig>> {
        let node = self.get_raft_handle(node_id)?;
        let metrics = node.metrics().borrow_watched().clone();
        Ok(metrics)
    }

    pub fn get_raft_handle(&self, node_id: &MemNodeId) -> Result<MemRaft, NetworkError<MemConfig>> {
        let rt = self.nodes.lock().unwrap();
        let node = rt
            .get(node_id)
            .ok_or_else(|| NetworkError::<MemConfig>::from_string(format!("node {} not found", *node_id)))?;
        let r = node.raft.clone();
        Ok(r)
    }

    pub fn get_storage_handle(&self, node_id: &MemNodeId) -> anyhow::Result<(MemLogStore, MemStateMachine)> {
        let rt = self.nodes.lock().unwrap();
        let node = rt.get(node_id).with_context(|| format!("could not find node {} in routing table", node_id))?;
        Ok((node.log_store.clone(), node.state_machine.clone()))
    }

    /// Set whether `limited_get_log_entries` should return empty results for a node.
    /// This is for testing graceful handling of faulty storage implementations.
    pub fn set_return_empty_limited_get(&self, node_id: &MemNodeId, value: bool) -> anyhow::Result<()> {
        let (log_store, _) = self.get_storage_handle(node_id)?;
        log_store.set_return_empty_limited_get(value);
        Ok(())
    }

    /// Set whether the next `limited_get_log_entries` call should fail for a node.
    pub fn set_fail_next_limited_get(&self, node_id: &MemNodeId, value: bool) -> anyhow::Result<()> {
        let (log_store, _) = self.get_storage_handle(node_id)?;
        log_store.set_fail_next_limited_get(value);
        Ok(())
    }

    /// Wait for a node's metrics with the shared finite deadline.
    pub fn wait(&self, node_id: &MemNodeId) -> Wait<MemConfig> {
        self.wait_with_timeout(node_id, WAIT_TIMEOUT)
    }

    /// Wait for a node's metrics within a specific observation window.
    pub fn wait_with_timeout(&self, node_id: &MemNodeId, duration: Duration) -> Wait<MemConfig> {
        let faults = self.fail_rpc.lock().unwrap();
        tracing::debug!(node_id, ?duration, ?faults, "waiting for node metrics");
        drop(faults);
        {
            let rules = self.rpc_rules.lock().unwrap();
            tracing::debug!(?rules, "active RPC fault rules");
        }
        let node = {
            let rt = self.nodes.lock().unwrap();
            let node = rt.get(node_id).expect("target node not found in routing table");
            node.raft.clone()
        };

        node.wait(Some(duration))
    }

    /// Wait until every requested node has applied at least `index`.
    /// One shared deadline covers the entire group; later applied indices also satisfy it.
    pub async fn wait_applied_at_least<I>(&self, ids: I, index: u64, message: &str) -> anyhow::Result<()>
    where I: IntoIterator<Item = MemNodeId> {
        let ids = ids.into_iter().collect::<Vec<_>>();
        let wait = async {
            for id in ids {
                self.wait(&id).applied_index_at_least(Some(index), message).await?;
            }
            Ok::<(), WaitError>(())
        };
        TypeConfig::timeout(WAIT_TIMEOUT, wait).await.context("group apply deadline exceeded")??;
        Ok(())
    }

    /// Get the ID of the current leader.
    pub fn leader(&self) -> Option<MemNodeId> {
        self.latest_metrics().into_iter().find_map(|node| {
            if node.current_leader == Some(node.id) {
                Some(node.id)
            } else {
                None
            }
        })
    }

    /// Bring up a new learner and add it to the leader's membership.
    ///
    /// Returns the full `RaftError`, including `Fatal` when the leader has stopped.
    pub async fn add_learner(
        &self,
        leader: MemNodeId,
        target: MemNodeId,
    ) -> Result<ClientWriteResponse<MemConfig>, RaftError<MemConfig, ClientWriteError<MemConfig>>> {
        let node = self.get_raft_handle(&leader).unwrap();
        let response = node.add_learner(target, (), true).await?;
        Ok(response)
    }

    /// Ensure read linearizability with user-specified policy.
    pub async fn ensure_linearizable(&self, target: MemNodeId, read_policy: ReadPolicy) -> anyhow::Result<()> {
        let n = self.get_raft_handle(&target)?;
        let linearizer = n.get_read_linearizer(read_policy).await?;
        linearizer.await_ready(&n).await?;
        Ok(())
    }

    /// Get the `ReadLogId` and last applied log ID.
    ///
    /// Returns the full `RaftError`, including `Fatal` when the node has stopped.
    pub async fn get_read_log_id(
        &self,
        target: MemNodeId,
        read_policy: ReadPolicy,
    ) -> Result<
        (ReadLogId<MemConfig>, Option<LogIdOf<MemConfig>>),
        RaftError<MemConfig, LinearizableReadError<MemConfig>>,
    > {
        let n = self.get_raft_handle(&target).unwrap();
        let read_state = n.get_read_log_id(read_policy).await?;
        Ok(read_state)
    }

    /// Write to exactly the requested node and preserve the acknowledged log ID.
    pub async fn client_request(
        &self,
        target: MemNodeId,
        client_id: &str,
        serial: u64,
    ) -> Result<ClientWriteResponse<MemConfig>, RaftError<MemConfig, ClientWriteError<MemConfig>>> {
        let request = ClientRequest::make_request(client_id, serial);
        self.write(target, request).await
    }

    /// Follow at most three leader redirects, returning the last error on exhaustion.
    pub async fn write_following_leader(
        &self,
        mut target: MemNodeId,
        request: ClientRequest,
    ) -> Result<ClientWriteResponse<MemConfig>, RaftError<MemConfig, ClientWriteError<MemConfig>>> {
        const MAX_REDIRECTS: usize = 3;
        for attempt in 0..=MAX_REDIRECTS {
            let error = match self.write(target, request.clone()).await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            let leader = error.forward_to_leader().and_then(|forward| forward.leader_id);
            let Some(leader) = leader else { return Err(error) };
            if attempt == MAX_REDIRECTS {
                return Err(error);
            }
            target = leader;
        }
        unreachable!("all redirect attempts return a result")
    }

    /// Send external request to the particular node.
    pub async fn with_raft_state<V, F>(&self, target: MemNodeId, func: F) -> Result<V, Fatal<MemConfig>>
    where
        F: FnOnce(&RaftState<MemConfig>) -> V + Send + 'static,
        V: Send + 'static,
    {
        let r = self.get_raft_handle(&target).unwrap();
        r.with_raft_state(func).await
    }

    /// Send external request to the particular node.
    pub async fn external_request<F: FnOnce(&RaftState<MemConfig>) + Send + 'static>(
        &self,
        target: MemNodeId,
        req: F,
    ) -> Result<(), Fatal<MemConfig>> {
        let r = self.get_raft_handle(&target).unwrap();
        r.external_request(req).await
    }

    /// Request the current leader from the target node.
    pub async fn current_leader(&self, target: MemNodeId) -> Option<MemNodeId> {
        let node = self.get_raft_handle(&target).unwrap();
        node.current_leader().await
    }

    /// Write a sequential workload, stopping at the first error.
    /// An empty workload has no acknowledged log ID.
    pub async fn client_request_many(
        &self,
        target: MemNodeId,
        client_id: &str,
        count: usize,
    ) -> Result<WriteSummary, RaftError<MemConfig, ClientWriteError<MemConfig>>> {
        let mut last_log_id = None;
        for serial in 0..count {
            let response = self.client_request(target, client_id, serial as u64).await?;
            last_log_id = Some(response.log_id);
        }
        Ok(WriteSummary {
            count: count as u64,
            last_log_id,
        })
    }

    /// Write to exactly the requested node and return the complete Raft response.
    pub async fn write(
        &self,
        target: MemNodeId,
        request: ClientRequest,
    ) -> Result<ClientWriteResponse<MemConfig>, RaftError<MemConfig, ClientWriteError<MemConfig>>> {
        let node = self.get_raft_handle(&target).expect("write target must be registered");
        node.client_write(request).await
    }

    #[allow(clippy::result_large_err)]
    pub fn emit_rpc_error<E>(&self, id: MemNodeId, target: MemNodeId) -> Result<(), RPCError<MemConfig, E>>
    where E: std::error::Error {
        let fails = self.fail_rpc.lock().unwrap();

        for key in [(id, NetSend), (target, NetRecv)] {
            if let Some(err_type) = fails.get(&key) {
                return Err(err_type.make_error(key.0, key.1));
            }
        }

        Ok(())
    }
}

/// Shared finite deadline for calls made directly to a Raft handle.
pub fn timeout() -> Option<Duration> {
    Some(WAIT_TIMEOUT)
}

/// Require a full timeout window; a stopped node fails the observation.
pub fn expect_timeout<T>(result: Result<T, WaitError>) -> anyhow::Result<()> {
    match result {
        Err(WaitError::Timeout(_, _)) => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => anyhow::bail!("expected a timeout, but the condition was met"),
    }
}

/// A trait for extending the functionality of a Raft vote.
///
/// This is used for testing.
pub trait TestingVoteExt: openraft::vote::RaftVote {
    /// Creates a new vote for a node in a specific term, with uncommitted status.
    fn from_term_node_id(
        term: <Self::LeaderId as openraft::vote::RaftLeaderId>::Term,
        node_id: <Self::LeaderId as openraft::vote::RaftLeaderId>::NodeId,
    ) -> Self {
        let leader_id = Self::LeaderId::new(term, node_id);
        Self::from_leader_id(leader_id, false)
    }

    /// Gets the node ID of the leader this vote is for.
    fn to_leader_node_id(&self) -> <Self::LeaderId as openraft::vote::RaftLeaderId>::NodeId {
        self.leader_node_id().clone()
    }

    /// Gets a reference to the node ID of the leader this vote is for.
    fn leader_node_id(&self) -> &<Self::LeaderId as openraft::vote::RaftLeaderId>::NodeId {
        self.leader_id().node_id()
    }
}

impl<T> TestingVoteExt for T where T: openraft::vote::RaftVote {}
