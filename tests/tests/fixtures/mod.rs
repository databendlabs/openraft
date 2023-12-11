//! Fixtures for testing Raft.

#![allow(dead_code)]

#[cfg(feature = "bt")] use std::backtrace::Backtrace;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::panic::PanicInfo;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Once;
use std::time::Duration;

use anyerror::AnyError;
use anyhow::Context;
use lazy_static::lazy_static;
use maplit::btreeset;
use openraft::async_trait::async_trait;
use openraft::error::CheckIsLeaderError;
use openraft::error::ClientWriteError;
use openraft::error::Fatal;
use openraft::error::Infallible;
use openraft::error::InstallSnapshotError;
use openraft::error::NetworkError;
use openraft::error::PayloadTooLarge;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::RemoteError;
use openraft::error::Unreachable;
use openraft::metrics::Wait;
use openraft::network::RaftNetwork;
use openraft::network::RaftNetworkFactory;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::ClientWriteResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::Adaptor;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftStateMachine;
use openraft::Config;
use openraft::LogId;
use openraft::LogIdOptionExt;
use openraft::MessageSummary;
use openraft::Node;
use openraft::NodeId;
use openraft::RPCTypes;
use openraft::Raft;
use openraft::RaftLogId;
use openraft::RaftMetrics;
use openraft::RaftState;
use openraft::RaftTypeConfig;
use openraft::ServerState;
use openraft::TokioInstant;
use openraft::TokioRuntime;
use openraft::Vote;
use openraft_memstore::ClientRequest;
use openraft_memstore::ClientResponse;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::MemNodeId;
use openraft_memstore::MemStore;
use openraft_memstore::TypeConfig;
use openraft_memstore::TypeConfig as MemConfig;
#[allow(unused_imports)] use pretty_assertions::assert_eq;
#[allow(unused_imports)] use pretty_assertions::assert_ne;
use tracing_appender::non_blocking::WorkerGuard;

use crate::fixtures::logging::init_file_logging;

pub mod logging;

pub type MemLogStore = Adaptor<MemConfig, Arc<MemStore>>;
pub type MemStateMachine = Adaptor<MemConfig, Arc<MemStore>>;

/// A concrete Raft type used during testing.
pub type MemRaft = Raft<MemConfig>;

pub fn init_default_ut_tracing() {
    static START: Once = Once::new();

    START.call_once(|| {
        let mut g = GLOBAL_UT_LOG_GUARD.as_ref().lock().unwrap();
        *g = Some(init_global_tracing("ut", "_log", "DEBUG"));
    });
}

lazy_static! {
    static ref GLOBAL_UT_LOG_GUARD: Arc<Mutex<Option<WorkerGuard>>> = Arc::new(Mutex::new(None));
}

pub fn init_global_tracing(app_name: &str, dir: &str, level: &str) -> WorkerGuard {
    set_panic_hook();

    let (g, sub) = init_file_logging(app_name, dir, level);
    tracing::subscriber::set_global_default(sub).expect("error setting global tracing subscriber");

    tracing::info!("initialized global tracing: in {}/{} at {}", dir, app_name, level);
    g
}

pub fn set_panic_hook() {
    std::panic::set_hook(Box::new(|panic| {
        log_panic(panic);
    }));
}

pub fn log_panic(panic: &PanicInfo) {
    let backtrace = {
        #[cfg(feature = "bt")]
        {
            format!("{:?}", Backtrace::force_capture())
        }

        #[cfg(not(feature = "bt"))]
        {
            "backtrace is disabled without --features 'bt'".to_string()
        }
    };

    eprintln!("{}", panic);

    if let Some(location) = panic.location() {
        tracing::error!(
            message = %panic,
            backtrace = %backtrace,
            panic.file = location.file(),
            panic.line = location.line(),
            panic.column = location.column(),
        );
        eprintln!("{}:{}:{}", location.file(), location.line(), location.column());
    } else {
        tracing::error!(message = %panic, backtrace = %backtrace);
    }

    eprintln!("{}", backtrace);
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

#[derive(Debug, Clone, Copy)]
pub enum RPCErrorType {
    /// Returns [`Unreachable`](`openraft::error::Unreachable`).
    Unreachable,
    /// Returns [`NetworkError`](`openraft::error::NetworkError`).
    NetworkError,
    /// Returns [`PayloadTooLarge`](`openraft::error::PayloadTooLarge`).
    PayloadTooLarge { action: RPCTypes, entries_hint: u64 },
}

impl RPCErrorType {
    fn make_error<NID, N, E>(&self, id: NID, dir: Direction) -> RPCError<NID, N, RaftError<NID, E>>
    where
        NID: NodeId,
        N: Node,
        E: std::error::Error,
    {
        let msg = format!("error {} id={}", dir, id);

        match self {
            RPCErrorType::Unreachable => Unreachable::new(&AnyError::error(msg)).into(),
            RPCErrorType::NetworkError => NetworkError::new(&AnyError::error(msg)).into(),
            RPCErrorType::PayloadTooLarge { action, entries_hint } => match action {
                RPCTypes::Vote => {
                    unreachable!("Vote RPC should not be too large")
                }
                RPCTypes::AppendEntries => PayloadTooLarge::new_entries_hint(*entries_hint).into(),
                RPCTypes::InstallSnapshot => {
                    unreachable!("InstallSnapshot RPC should not be too large")
                }
            },
        }
    }
}

/// Pre-hook result, which does not return remote Error.
pub type PreHookResult = Result<(), RPCError<MemNodeId, (), Infallible>>;

#[derive(derive_more::From, derive_more::TryInto)]
pub enum RPCRequest<C: RaftTypeConfig> {
    AppendEntries(AppendEntriesRequest<C>),
    InstallSnapshot(InstallSnapshotRequest<C>),
    Vote(VoteRequest<C::NodeId>),
}

impl<C: RaftTypeConfig> RPCRequest<C> {
    pub fn get_type(&self) -> RPCTypes {
        match self {
            RPCRequest::AppendEntries(_) => RPCTypes::AppendEntries,
            RPCRequest::InstallSnapshot(_) => RPCTypes::InstallSnapshot,
            RPCRequest::Vote(_) => RPCTypes::Vote,
        }
    }
}

/// Arguments: `(router, rpc, from_id, to_id)`
pub type RPCPreHook =
    Box<dyn Fn(&TypedRaftRouter, RPCRequest<TypeConfig>, MemNodeId, MemNodeId) -> PreHookResult + Send + 'static>;

/// A type which emulates a network transport and implements the `RaftNetworkFactory` trait.
#[derive(Clone)]
pub struct TypedRaftRouter {
    /// The Raft runtime config which all nodes are using.
    config: Arc<Config>,

    /// The table of all nodes currently known to this router instance.
    #[allow(clippy::type_complexity)]
    nodes: Arc<Mutex<BTreeMap<MemNodeId, (MemRaft, MemLogStore, MemStateMachine)>>>,

    /// Whether to fail a network RPC that is sent from/to a node.
    /// And it defines what kind of error to return.
    fail_rpc: Arc<Mutex<HashMap<(MemNodeId, Direction), RPCErrorType>>>,

    /// To emulate network delay for sending, in milliseconds.
    /// 0 means no delay.
    send_delay: Arc<AtomicU64>,

    /// To simulate PartialSuccess for AppendEntries RPCs.
    ///
    /// If the quota is set to `Some(n)`, then the AppendEntries RPC consumes the quota,
    /// and send out at most `n` entries.
    append_entries_quota: Arc<Mutex<Option<u64>>>,

    /// Count of RPCs sent.
    rpc_count: Arc<Mutex<HashMap<RPCTypes, u64>>>,

    /// A hook function to be called when before an RPC is sent to target node.
    rpc_pre_hook: Arc<Mutex<HashMap<RPCTypes, RPCPreHook>>>,
}

/// Default `RaftRouter` for memstore.
pub type RaftRouter = TypedRaftRouter;

pub struct Builder {
    config: Arc<Config>,
    send_delay: u64,
}

impl Builder {
    pub fn send_delay(mut self, ms: u64) -> Self {
        self.send_delay = ms;
        self
    }

    pub fn build(self) -> TypedRaftRouter {
        let send_delay = {
            let send_delay = env::var("OPENRAFT_NETWORK_SEND_DELAY").ok();

            if let Some(d) = send_delay {
                tracing::info!("OPENRAFT_NETWORK_SEND_DELAY set send-delay to {} ms", d);
                d.parse::<u64>().unwrap()
            } else {
                self.send_delay
            }
        };
        TypedRaftRouter {
            config: self.config,
            nodes: Default::default(),
            fail_rpc: Default::default(),
            send_delay: Arc::new(AtomicU64::new(send_delay)),
            append_entries_quota: Arc::new(Mutex::new(None)),
            rpc_count: Default::default(),
            rpc_pre_hook: Default::default(),
        }
    }
}

impl TypedRaftRouter {
    pub fn builder(config: Arc<Config>) -> Builder {
        Builder { config, send_delay: 0 }
    }

    /// Create a new instance.
    pub fn new(config: Arc<Config>) -> Self {
        Self::builder(config).build()
    }

    pub fn network_send_delay(&mut self, ms: u64) {
        self.send_delay.store(ms, Ordering::Relaxed);
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn rand_send_delay(&self) {
        let send_delay = self.send_delay.load(Ordering::Relaxed);
        if send_delay == 0 {
            return;
        }

        let r = rand::random::<u64>() % send_delay;
        let timeout = Duration::from_millis(r);
        tokio::time::sleep(timeout).await;
    }

    pub fn set_append_entries_quota(&mut self, quota: Option<u64>) {
        let mut append_entries_quota = self.append_entries_quota.lock().unwrap();
        *append_entries_quota = quota;
    }

    fn count_rpc(&self, rpc_type: RPCTypes) {
        let mut rpc_count = self.rpc_count.lock().unwrap();
        let count = rpc_count.entry(rpc_type).or_insert(0);
        *count += 1;
    }

    pub fn get_rpc_count(&self) -> HashMap<RPCTypes, u64> {
        self.rpc_count.lock().unwrap().clone()
    }

    /// Create a cluster: 0 is the initial leader, others are voters and learners
    ///
    /// NOTE: it create a single node cluster first, then change it to a multi-voter cluster.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn new_cluster(
        &mut self,
        voter_ids: BTreeSet<MemNodeId>,
        learners: BTreeSet<MemNodeId>,
    ) -> anyhow::Result<u64> {
        let leader_id = MemNodeId::default();
        assert!(voter_ids.contains(&leader_id));

        self.new_raft_node(leader_id).await;

        tracing::info!("--- wait for init node to ready");

        // Sending an external requests will also find all nodes in Learner state.
        //
        // This demonstrates fire-and-forget external request, which will be serialized
        // with other processing. It is not required for the correctness of the test
        //
        // Since the execution of API messages is serialized, even if the request executes
        // some unknown time in the future (due to fire-and-forget semantics), it will
        // properly receive the state before initialization, as that state will appear
        // later in the sequence.
        //
        // Also, this external request will be definitely executed, since it's ordered
        // before other requests in the Raft core API queue, which definitely are executed
        // (since they are awaited).
        #[allow(clippy::single_element_loop)]
        for node in [0] {
            self.external_request(node, |s| {
                assert_eq!(s.server_state, ServerState::Learner);
            });
        }
        self.wait_for_log(&btreeset![leader_id], None, timeout(), "empty").await?;

        tracing::info!("--- initializing single node cluster: {}", 0);

        self.initialize(leader_id).await?;
        let mut log_index = 1; // log 0: initial membership log; log 1: leader initial log

        tracing::info!(log_index, "--- wait for init node to become leader");

        self.wait_for_log(&btreeset![leader_id], Some(log_index), timeout(), "init").await?;
        self.wait(&leader_id, timeout()).vote(Vote::new_committed(1, 0), "init vote").await?;

        for id in voter_ids.iter() {
            if *id == leader_id {
                continue;
            }
            tracing::info!(log_index, "--- add voter: {}", id);

            self.new_raft_node(*id).await;
            self.add_learner(leader_id, *id).await?;
            log_index += 1;

            self.wait_for_state(&btreeset![*id], ServerState::Learner, timeout(), "empty node").await?;
        }

        self.wait_for_log(
            &voter_ids,
            Some(log_index),
            timeout(),
            &format!("learners of {:?}", voter_ids),
        )
        .await?;

        if voter_ids.len() > 1 {
            tracing::info!(log_index, "--- change membership to setup voters: {:?}", voter_ids);

            let node = self.get_raft_handle(&MemNodeId::default())?;
            node.change_membership(voter_ids.clone(), false).await?;
            log_index += 2;

            self.wait_for_log(
                &voter_ids,
                Some(log_index),
                timeout(),
                &format!("cluster of {:?}", voter_ids),
            )
            .await?;
        }

        for id in learners.clone() {
            tracing::info!(log_index, "--- add learner: {}", id);
            self.new_raft_node(id).await;
            self.add_learner(MemNodeId::default(), id).await?;
            log_index += 1;
        }
        self.wait_for_log(
            &learners,
            Some(log_index),
            timeout(),
            &format!("learners of {:?}", learners),
        )
        .await?;

        Ok(log_index)
    }

    /// Create and register a new Raft node bearing the given ID.
    pub async fn new_raft_node(&mut self, id: MemNodeId) {
        let (log_store, sm) = self.new_store();
        self.new_raft_node_with_sto(id, log_store, sm).await
    }

    pub fn new_store(&mut self) -> (MemLogStore, MemStateMachine) {
        let store = Arc::new(MemStore::default());
        Adaptor::new(store)
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn new_raft_node_with_sto(&mut self, id: MemNodeId, log_store: MemLogStore, sm: MemStateMachine) {
        let node = Raft::new(id, self.config.clone(), self.clone(), log_store.clone(), sm.clone()).await.unwrap();
        let mut rt = self.nodes.lock().unwrap();
        rt.insert(id, (node, log_store, sm));
    }

    /// Remove the target node from the routing table & isolation.
    pub fn remove_node(&mut self, id: MemNodeId) -> Option<(MemRaft, MemLogStore, MemStateMachine)> {
        let opt_handles = {
            let mut rt = self.nodes.lock().unwrap();
            rt.remove(&id)
        };

        self.set_network_error(id, false);
        self.set_unreachable(id, false);

        opt_handles
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
            Some(RPCErrorType::NetworkError)
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
            Some(RPCErrorType::Unreachable)
        } else {
            None
        };
        self.set_rpc_failure(id, NetRecv, v);
        self.set_rpc_failure(id, NetSend, v);
    }

    /// Set whether to emit a specified rpc error when sending to/receiving from a node.
    pub fn set_rpc_failure(&self, id: MemNodeId, dir: Direction, rpc_error_type: Option<RPCErrorType>) {
        let mut fails = self.fail_rpc.lock().unwrap();
        if let Some(rpc_error_type) = rpc_error_type {
            fails.insert((id, dir), rpc_error_type);
        } else {
            fails.remove(&(id, dir));
        }
    }

    /// Set a hook function to be called when before an RPC is sent to target node.
    pub fn set_rpc_pre_hook<F>(&self, rpc_type: RPCTypes, hook: F)
    where F: Fn(&TypedRaftRouter, RPCRequest<TypeConfig>, MemNodeId, MemNodeId) -> PreHookResult + Send + 'static {
        self.rpc_pre_hook(rpc_type, Some(Box::new(hook)));
    }

    /// Set or unset a hook function to be called when before an RPC is sent to target node.
    pub fn rpc_pre_hook(&self, rpc_type: RPCTypes, hook: Option<RPCPreHook>) {
        let mut rpc_pre_hook = self.rpc_pre_hook.lock().unwrap();
        if let Some(hook) = hook {
            rpc_pre_hook.insert(rpc_type, hook);
        } else {
            rpc_pre_hook.remove(&rpc_type);
        }
    }

    /// Call pre-hook before an RPC is sent.
    fn call_rpc_pre_hook<E>(
        &self,
        request: impl Into<RPCRequest<TypeConfig>>,
        from: MemNodeId,
        to: MemNodeId,
    ) -> Result<(), RPCError<MemNodeId, (), E>>
    where
        E: std::error::Error,
    {
        let request = request.into();
        let typ = request.get_type();

        let rpc_pre_hook = self.rpc_pre_hook.lock().unwrap();

        if let Some(hook) = rpc_pre_hook.get(&typ) {
            let res = hook(self, request, from, to);
            match res {
                Ok(()) => Ok(()),
                Err(err) => {
                    // The pre-hook should only return RPCError variants
                    let rpc_err = match err {
                        RPCError::Timeout(e) => e.into(),
                        RPCError::Unreachable(e) => e.into(),
                        RPCError::PayloadTooLarge(e) => e.into(),
                        RPCError::Network(e) => e.into(),
                        RPCError::RemoteError(e) => {
                            unreachable!("unexpected RemoteError: {:?}", e);
                        }
                    };
                    Err(rpc_err)
                }
            }
        } else {
            Ok(())
        }
    }

    /// Get a payload of the latest metrics from each node in the cluster.
    #[allow(clippy::significant_drop_in_scrutinee)]
    pub fn latest_metrics(&self) -> Vec<RaftMetrics<MemNodeId, ()>> {
        let rt = self.nodes.lock().unwrap();
        let mut metrics = vec![];
        for node in rt.values() {
            let m = node.0.metrics().borrow().clone();
            tracing::debug!("router::latest_metrics: {:?}", m);
            metrics.push(m);
        }
        metrics
    }

    pub fn get_metrics(&self, node_id: &MemNodeId) -> anyhow::Result<RaftMetrics<MemNodeId, ()>> {
        let node = self.get_raft_handle(node_id)?;
        let metrics = node.metrics().borrow().clone();
        Ok(metrics)
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub fn get_raft_handle(&self, node_id: &MemNodeId) -> Result<MemRaft, NetworkError> {
        let rt = self.nodes.lock().unwrap();
        let raft_and_sto = rt
            .get(node_id)
            .ok_or_else(|| NetworkError::new(&AnyError::error(format!("node {} not found", *node_id))))?;
        let r = raft_and_sto.clone().0;
        Ok(r)
    }

    pub fn get_storage_handle(&self, node_id: &MemNodeId) -> anyhow::Result<(MemLogStore, MemStateMachine)> {
        let rt = self.nodes.lock().unwrap();
        let addr = rt.get(node_id).with_context(|| format!("could not find node {} in routing table", node_id))?;
        let x = addr.clone();
        Ok((x.1, x.2))
    }

    /// Wait for metrics until it satisfies some condition.
    #[tracing::instrument(level = "info", skip(self, func))]
    pub async fn wait_for_metrics<T>(
        &self,
        node_id: &MemNodeId,
        func: T,
        timeout: Option<Duration>,
        msg: &str,
    ) -> anyhow::Result<RaftMetrics<MemNodeId, ()>>
    where
        T: Fn(&RaftMetrics<MemNodeId, ()>) -> bool + Send,
    {
        let wait = self.wait(node_id, timeout);
        let rst = wait.metrics(func, format!("node-{} {}", node_id, msg)).await?;
        Ok(rst)
    }

    pub fn wait(&self, node_id: &MemNodeId, timeout: Option<Duration>) -> Wait<MemNodeId, (), TokioRuntime> {
        let node = {
            let rt = self.nodes.lock().unwrap();
            rt.get(node_id).expect("target node not found in routing table").clone().0
        };

        node.wait(timeout)
    }

    /// Wait for specified nodes until they applied upto `want_log`(inclusive) logs.
    #[tracing::instrument(level = "info", skip(self))]
    pub async fn wait_for_log(
        &self,
        node_ids: &BTreeSet<MemNodeId>,
        want_log: Option<u64>,
        timeout: Option<Duration>,
        msg: &str,
    ) -> anyhow::Result<()> {
        for i in node_ids.iter() {
            self.wait(i, timeout).applied_index(want_log, msg).await?;
        }
        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self))]
    pub async fn wait_for_members(
        &self,
        node_ids: &BTreeSet<MemNodeId>,
        members: BTreeSet<MemNodeId>,
        timeout: Option<Duration>,
        msg: &str,
    ) -> anyhow::Result<()> {
        for i in node_ids.iter() {
            let wait = self.wait(i, timeout);
            wait.metrics(
                |x| x.membership_config.voter_ids().collect::<BTreeSet<MemNodeId>>() == members,
                msg,
            )
            .await?;
        }
        Ok(())
    }

    /// Wait for specified nodes until their state becomes `state`.
    #[tracing::instrument(level = "info", skip(self))]
    pub async fn wait_for_state(
        &self,
        node_ids: &BTreeSet<MemNodeId>,
        want_state: ServerState,
        timeout: Option<Duration>,
        msg: &str,
    ) -> anyhow::Result<()> {
        for i in node_ids.iter() {
            self.wait(i, timeout).state(want_state, msg).await?;
        }
        Ok(())
    }

    /// Wait for specified nodes until their snapshot becomes `want`.
    #[tracing::instrument(level = "info", skip(self))]
    pub async fn wait_for_snapshot(
        &self,
        node_ids: &BTreeSet<MemNodeId>,
        want: LogId<MemNodeId>,
        timeout: Option<Duration>,
        msg: &str,
    ) -> anyhow::Result<()> {
        for i in node_ids.iter() {
            self.wait(i, timeout).snapshot(want, msg).await?;
        }
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
    pub async fn add_learner(
        &self,
        leader: MemNodeId,
        target: MemNodeId,
    ) -> Result<ClientWriteResponse<MemConfig>, ClientWriteError<MemNodeId, ()>> {
        let node = self.get_raft_handle(&leader).unwrap();
        node.add_learner(target, (), true).await.map_err(|e| e.into_api_error().unwrap())
    }

    /// Ensure read linearizability.
    pub async fn ensure_linearizable(&self, target: MemNodeId) -> Result<(), CheckIsLeaderError<MemNodeId, ()>> {
        let n = self.get_raft_handle(&target).unwrap();
        n.ensure_linearizable().await.map_err(|e| e.into_api_error().unwrap())?;
        Ok(())
    }

    /// Send a client request to the target node, causing test failure on error.
    pub async fn client_request(
        &self,
        mut target: MemNodeId,
        client_id: &str,
        serial: u64,
    ) -> Result<(), RaftError<MemNodeId, ClientWriteError<MemNodeId, ()>>> {
        for ith in 0..3 {
            let req = ClientRequest::make_request(client_id, serial);
            if let Err(err) = self.send_client_request(target, req).await {
                tracing::error!({error=%err}, "error from client request");

                #[allow(clippy::single_match)]
                match &err {
                    RaftError::APIError(ClientWriteError::ForwardToLeader(e)) => {
                        tracing::info!(
                            "{}-th request: target is not leader anymore. New leader is: {:?}",
                            ith,
                            e.leader_id
                        );
                        if let Some(l) = e.leader_id {
                            target = l;
                            continue;
                        }
                    }
                    _ => {}
                }
                return Err(err);
            } else {
                return Ok(());
            }
        }

        unreachable!(
            "Max retry times exceeded. Can not finish client_request, target={}, client_id={} serial={}",
            target, client_id, serial
        )
    }

    /// Send external request to the particular node.
    pub async fn with_raft_state<V, F>(&self, target: MemNodeId, func: F) -> Result<V, Fatal<MemNodeId>>
    where
        F: FnOnce(&RaftState<MemNodeId, (), TokioInstant>) -> V + Send + 'static,
        V: Send + 'static,
    {
        let r = self.get_raft_handle(&target).unwrap();
        r.with_raft_state(func).await
    }

    /// Send external request to the particular node.
    pub fn external_request<F: FnOnce(&RaftState<MemNodeId, (), TokioInstant>) + Send + 'static>(
        &self,
        target: MemNodeId,
        req: F,
    ) {
        let r = self.get_raft_handle(&target).unwrap();
        r.external_request(req)
    }

    /// Request the current leader from the target node.
    pub async fn current_leader(&self, target: MemNodeId) -> Option<MemNodeId> {
        let node = self.get_raft_handle(&target).unwrap();
        node.current_leader().await
    }

    /// Send multiple client requests to the target node, causing test failure on error.
    /// Returns the number of log written to raft.
    pub async fn client_request_many(
        &self,
        target: MemNodeId,
        client_id: &str,
        count: usize,
    ) -> Result<u64, RaftError<MemNodeId, ClientWriteError<MemNodeId, ()>>> {
        for idx in 0..count {
            self.client_request(target, client_id, idx as u64).await?;
        }

        Ok(count as u64)
    }

    pub async fn send_client_request(
        &self,
        target: MemNodeId,
        req: ClientRequest,
    ) -> Result<ClientResponse, RaftError<MemNodeId, ClientWriteError<MemNodeId, ()>>> {
        let node = {
            let rt = self.nodes.lock().unwrap();
            rt.get(&target)
                .unwrap_or_else(|| panic!("node '{}' does not exist in routing table", target))
                .clone()
        };

        node.0.client_write(req).await.map(|res| res.data)
    }

    /// Assert against the state of the storage system one node in the cluster.
    pub async fn assert_storage_state_with_sto(
        &self,
        storage: &mut MemLogStore,
        sm: &mut MemStateMachine,
        id: &MemNodeId,
        expect_term: u64,
        expect_last_log: u64,
        expect_voted_for: Option<MemNodeId>,
        expect_sm_last_applied_log: LogId<MemNodeId>,
        expect_snapshot: &Option<(ValueTest<u64>, u64)>,
    ) -> anyhow::Result<()> {
        let last_log_id = storage.get_log_state().await?.last_log_id;

        assert_eq!(
            expect_last_log,
            last_log_id.index().unwrap(),
            "expected node {} to have last_log {}, got {:?}",
            id,
            expect_last_log,
            last_log_id
        );

        let vote = storage.read_vote().await?.unwrap_or_else(|| panic!("no hard state found for node {}", id));

        assert_eq!(
            vote.leader_id().get_term(),
            expect_term,
            "expected node {} to have term {}, got {:?}",
            id,
            expect_term,
            vote
        );

        if let Some(voted_for) = &expect_voted_for {
            assert_eq!(
                vote.leader_id().voted_for(),
                Some(*voted_for),
                "expected node {} to have voted for {}, got {:?}",
                id,
                voted_for,
                vote
            );
        }

        if let Some((index_test, term)) = &expect_snapshot {
            let snap = sm
                .get_current_snapshot()
                .await
                .map_err(|err| panic!("{}", err))
                .unwrap()
                .unwrap_or_else(|| panic!("no snapshot present for node {}", id));

            match index_test {
                ValueTest::Exact(index) => assert_eq!(
                    snap.meta.last_log_id.index(),
                    Some(*index),
                    "expected node {} to have snapshot with index {}, got {:?}",
                    id,
                    index,
                    snap.meta.last_log_id
                ),
                ValueTest::Range(range) => assert!(
                    range.contains(&snap.meta.last_log_id.index().unwrap_or_default()),
                    "expected node {} to have snapshot within range {:?}, got {:?}",
                    id,
                    range,
                    snap.meta.last_log_id
                ),
            }

            assert_eq!(
                &snap.meta.last_log_id.unwrap_or_default().leader_id.term,
                term,
                "expected node {} to have snapshot with term {}, got {:?}",
                id,
                term,
                snap.meta.last_log_id
            );
        }

        let (last_applied, _) = sm.applied_state().await?;

        assert_eq!(
            &last_applied,
            &Some(expect_sm_last_applied_log),
            "expected node {} to have state machine last_applied_log {}, got {:?}",
            id,
            expect_sm_last_applied_log,
            last_applied
        );

        Ok(())
    }

    /// Assert against the state of the storage system per node in the cluster.
    pub async fn assert_storage_state(
        &self,
        expect_term: u64,
        expect_last_log: u64,
        expect_voted_for: Option<MemNodeId>,
        expect_sm_last_applied_log: LogId<MemNodeId>,
        expect_snapshot: Option<(ValueTest<u64>, u64)>,
    ) -> anyhow::Result<()> {
        let node_ids = {
            let rt = self.nodes.lock().unwrap();
            let node_ids = rt.keys().cloned().collect::<Vec<_>>();
            node_ids
        };

        for id in node_ids {
            let (mut storage, mut sm) = self.get_storage_handle(&id)?;

            self.assert_storage_state_with_sto(
                &mut storage,
                &mut sm,
                &id,
                expect_term,
                expect_last_log,
                expect_voted_for,
                expect_sm_last_applied_log,
                &expect_snapshot,
            )
            .await?;
        }

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub fn emit_rpc_error<E>(
        &self,
        id: MemNodeId,
        target: MemNodeId,
    ) -> Result<(), RPCError<MemNodeId, (), RaftError<MemNodeId, E>>>
    where
        E: std::error::Error,
    {
        let fails = self.fail_rpc.lock().unwrap();

        for key in [(id, NetSend), (target, NetRecv)] {
            if let Some(err_type) = fails.get(&key) {
                return Err(err_type.make_error(key.0, key.1));
            }
        }

        Ok(())
    }
}

#[async_trait]
impl RaftNetworkFactory<MemConfig> for TypedRaftRouter {
    type Network = RaftRouterNetwork;

    async fn new_client(&mut self, target: MemNodeId, _node: &()) -> Self::Network {
        RaftRouterNetwork {
            target,
            owner: self.clone(),
        }
    }
}

pub struct RaftRouterNetwork {
    target: MemNodeId,
    owner: TypedRaftRouter,
}

#[async_trait]
impl RaftNetwork<MemConfig> for RaftRouterNetwork {
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn send_append_entries(
        &mut self,
        mut rpc: AppendEntriesRequest<MemConfig>,
    ) -> Result<AppendEntriesResponse<MemNodeId>, RPCError<MemNodeId, (), RaftError<MemNodeId>>> {
        let from_id = rpc.vote.leader_id().voted_for().unwrap();

        tracing::debug!("append_entries to id={} {}", self.target, rpc.summary());
        self.owner.count_rpc(RPCTypes::AppendEntries);
        self.owner.call_rpc_pre_hook(rpc.clone(), from_id, self.target)?;
        self.owner.emit_rpc_error(from_id, self.target)?;
        self.owner.rand_send_delay().await;

        // decrease quota if quota is set
        let truncated = {
            let n = rpc.entries.len() as u64;

            let mut x = self.owner.append_entries_quota.lock().unwrap();
            let q = *x;
            tracing::debug!("current quota: {:?}", q);

            if let Some(quota) = q {
                if quota < n {
                    rpc.entries.truncate(quota as usize);
                    *x = Some(0);
                    if let Some(last) = rpc.entries.last() {
                        Some(Some(*last.get_log_id()))
                    } else {
                        Some(rpc.prev_log_id)
                    }
                } else {
                    *x = Some(quota - n);
                    None
                }
            } else {
                None
            }
        };

        {
            let x = self.owner.append_entries_quota.lock().unwrap();
            tracing::debug!("quota after consumption: {:?}", *x);
        }
        tracing::debug!("append_entries truncated: {:?}", truncated);

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.append_entries(rpc).await;

        tracing::debug!("append_entries: recv resp from id={} {:?}", self.target, resp);
        let resp = resp.map_err(|e| RemoteError::new(self.target, e))?;

        // If entries are truncated by quota, return an partial success response.
        if let Some(truncated) = truncated {
            match resp {
                AppendEntriesResponse::Success => Ok(AppendEntriesResponse::PartialSuccess(truncated)),
                _ => Ok(resp),
            }
        } else {
            Ok(resp)
        }
    }

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    async fn send_install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<MemConfig>,
    ) -> Result<InstallSnapshotResponse<MemNodeId>, RPCError<MemNodeId, (), RaftError<MemNodeId, InstallSnapshotError>>>
    {
        let from_id = rpc.vote.leader_id().voted_for().unwrap();

        self.owner.count_rpc(RPCTypes::InstallSnapshot);
        self.owner.call_rpc_pre_hook(rpc.clone(), from_id, self.target)?;
        self.owner.emit_rpc_error(from_id, self.target)?;
        self.owner.rand_send_delay().await;

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.install_snapshot(rpc).await;
        let resp = resp.map_err(|e| RemoteError::new(self.target, e))?;

        Ok(resp)
    }

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn send_vote(
        &mut self,
        rpc: VoteRequest<MemNodeId>,
    ) -> Result<VoteResponse<MemNodeId>, RPCError<MemNodeId, (), RaftError<MemNodeId>>> {
        let from_id = rpc.vote.leader_id().voted_for().unwrap();

        self.owner.count_rpc(RPCTypes::Vote);
        self.owner.call_rpc_pre_hook(rpc.clone(), from_id, self.target)?;
        self.owner.emit_rpc_error(from_id, self.target)?;
        self.owner.rand_send_delay().await;

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.vote(rpc).await;
        let resp = resp.map_err(|e| RemoteError::new(self.target, e))?;

        Ok(resp)
    }
}

pub enum ValueTest<T> {
    Exact(T),
    Range(std::ops::Range<T>),
}

impl<T> From<T> for ValueTest<T> {
    fn from(src: T) -> Self {
        Self::Exact(src)
    }
}

impl<T> From<std::ops::Range<T>> for ValueTest<T> {
    fn from(src: std::ops::Range<T>) -> Self {
        Self::Range(src)
    }
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5_000))
}
