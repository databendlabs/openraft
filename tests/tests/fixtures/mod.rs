//! Fixtures for testing Raft.

#![allow(dead_code)]
#![allow(clippy::uninlined_format_args)]

#[cfg(feature = "bt")]
use std::backtrace::Backtrace;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::future::Future;
use std::panic::PanicHookInfo;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Once;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyerror::AnyError;
use anyhow::Context;
use lazy_static::lazy_static;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::OptionalSend;
use openraft::RPCTypes;
use openraft::Raft;
use openraft::RaftLogReader;
use openraft::RaftMetrics;
use openraft::RaftState;
use openraft::RaftTypeConfig;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::Vote;
use openraft::error::CheckIsLeaderError;
use openraft::error::ClientWriteError;
use openraft::error::Fatal;
use openraft::error::Infallible;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::error::Unreachable;
use openraft::metrics::Wait;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::ClientWriteResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot;
use openraft_memstore::ClientRequest;
use openraft_memstore::ClientResponse;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::MemLogStore as LogStoreInner;
use openraft_memstore::MemNodeId;
use openraft_memstore::MemStateMachine as SMInner;
use openraft_memstore::TypeConfig;
use openraft_memstore::TypeConfig as MemConfig;
#[allow(unused_imports)]
use pretty_assertions::assert_eq;
#[allow(unused_imports)]
use pretty_assertions::assert_ne;
use tracing_appender::non_blocking::WorkerGuard;

use crate::fixtures::logging::init_file_logging;

pub mod logging;

pub type MemLogStore = Arc<LogStoreInner>;
pub type MemStateMachine = Arc<SMInner>;

/// A concrete Raft type used during testing.
pub type MemRaft = Raft<MemConfig>;

pub fn log_id(term: u64, node_id: u64, index: u64) -> LogIdOf<TypeConfig> {
    LogIdOf::<TypeConfig>::new(
        <TypeConfig as RaftTypeConfig>::LeaderId::new_committed(term, node_id),
        index,
    )
}

/// Create a harness that sets up tracing and a tokio runtime for testing.
pub fn ut_harness<F, Fut>(f: F) -> anyhow::Result<()>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = anyhow::Result<()>> + 'static,
{
    fn func_name<F: std::any::Any>() -> &'static str {
        let full_name = std::any::type_name::<F>();
        full_name.rsplit("::").find(|name| *name != "{{closure}}").unwrap()
    }

    #[allow(clippy::let_unit_value)]
    let _g = init_default_ut_tracing();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .expect("Failed building the Runtime");

    let res = rt.block_on(f());
    if let Err(e) = &res {
        tracing::error!("{} error: {:?}", func_name::<F>(), e);
    }
    res
}

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
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        log_panic(panic);
        prev_hook(panic);
    }));
}

pub fn log_panic(panic: &PanicHookInfo) {
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

    if let Some(location) = panic.location() {
        tracing::error!(
            message = %panic.to_string().replace('\n', " "),
            backtrace = %backtrace,
            panic.file = location.file(),
            panic.line = location.line(),
            panic.column = location.column(),
        );
    } else {
        tracing::error!(message = %panic.to_string().replace('\n', " "), backtrace = %backtrace);
    }
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
use openraft::entry::RaftEntry;
use openraft::network::v2::RaftNetworkV2;
use openraft::vote::RaftLeaderId;
use openraft::vote::RaftLeaderIdExt;
use openraft::vote::RaftVote;

#[derive(Debug, Clone, Copy)]
pub enum RPCErrorType {
    /// Returns [`Unreachable`](`openraft::error::Unreachable`).
    Unreachable,
    /// Returns [`NetworkError`](`openraft::error::NetworkError`).
    NetworkError,
}

impl RPCErrorType {
    fn make_error<C>(&self, id: C::NodeId, dir: Direction) -> RPCError<C>
    where C: RaftTypeConfig {
        let msg = format!("error {} id={}", dir, id);

        match self {
            RPCErrorType::Unreachable => Unreachable::new(&AnyError::error(msg)).into(),
            RPCErrorType::NetworkError => NetworkError::new(&AnyError::error(msg)).into(),
        }
    }
}

/// Pre-hook result, which does not return remote Error.
pub type PreHookResult = Result<(), RPCError<MemConfig, Infallible>>;

#[derive(Debug)]
#[derive(derive_more::From, derive_more::TryInto)]
pub enum RPCRequest<C: RaftTypeConfig>
where C::SnapshotData: fmt::Debug
{
    AppendEntries(AppendEntriesRequest<C>),
    InstallSnapshot(InstallSnapshotRequest<C>),
    InstallFullSnapshot(Snapshot<C>),
    Vote(VoteRequest<C>),
    TransferLeader(TransferLeaderRequest<C>),
}

impl<C: RaftTypeConfig> RPCRequest<C>
where C::SnapshotData: fmt::Debug
{
    pub fn get_type(&self) -> RPCTypes {
        match self {
            RPCRequest::AppendEntries(_) => RPCTypes::AppendEntries,
            RPCRequest::InstallSnapshot(_) => RPCTypes::InstallSnapshot,
            RPCRequest::InstallFullSnapshot(_) => RPCTypes::InstallSnapshot,
            RPCRequest::Vote(_) => RPCTypes::Vote,
            RPCRequest::TransferLeader(_) => RPCTypes::TransferLeader,
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

    /// Whether to save the committed entries to the RaftLogStorage.
    pub enable_saving_committed: bool,

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
            enable_saving_committed: true,
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
            })
            .await?;
        }
        self.wait(&leader_id, timeout()).applied_index(None, "empty").await?;

        tracing::info!("--- initializing single node cluster: {}", 0);

        self.initialize(leader_id).await?;
        let mut log_index = 1; // log 0: initial membership log; log 1: leader initial log

        tracing::info!(log_index, "--- wait for init node to become leader");

        self.wait(&leader_id, timeout()).applied_index(Some(log_index), "init").await?;
        self.wait(&leader_id, timeout()).vote(VoteOf::<MemConfig>::new_committed(1, 0), "init vote").await?;

        for id in voter_ids.iter() {
            if *id == leader_id {
                continue;
            }
            tracing::info!(log_index, "--- add voter: {}", id);

            self.new_raft_node(*id).await;
            self.add_learner(leader_id, *id).await?;
            log_index += 1;

            self.wait(id, timeout()).state(ServerState::Learner, "empty node").await?;
        }

        for id in voter_ids.iter() {
            self.wait(id, timeout())
                .applied_index(Some(log_index), &format!("learners of {:?}", voter_ids))
                .await?;
        }

        if voter_ids.len() > 1 {
            tracing::info!(log_index, "--- change membership to setup voters: {:?}", voter_ids);

            let node = self.get_raft_handle(&MemNodeId::default())?;
            node.change_membership(voter_ids.clone(), false).await?;
            log_index += 2;

            for id in voter_ids.iter() {
                self.wait(id, timeout())
                    .applied_index(Some(log_index), &format!("cluster of {:?}", voter_ids))
                    .await?;
            }
        }

        for id in learners.clone() {
            tracing::info!(log_index, "--- add learner: {}", id);
            self.new_raft_node(id).await;
            self.add_learner(MemNodeId::default(), id).await?;
            log_index += 1;
        }
        for id in learners.iter() {
            self.wait(id, timeout())
                .applied_index(Some(log_index), &format!("learners of {:?}", learners))
                .await?;
        }

        Ok(log_index)
    }

    /// Create and register a new Raft node bearing the given ID.
    pub async fn new_raft_node(&mut self, id: MemNodeId) {
        let (log_store, sm) = self.new_store();
        self.new_raft_node_with_sto(id, log_store, sm).await
    }

    pub fn new_store(&mut self) -> (MemLogStore, MemStateMachine) {
        let (log, sm) = openraft_memstore::new_mem_store();
        log.enable_saving_committed.store(self.enable_saving_committed, Ordering::Relaxed);
        (log, sm)
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
    #[allow(clippy::result_large_err)]
    fn call_rpc_pre_hook<E>(
        &self,
        request: impl Into<RPCRequest<TypeConfig>>,
        from: MemNodeId,
        to: MemNodeId,
    ) -> Result<(), RPCError<MemConfig, E>>
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
    pub fn latest_metrics(&self) -> Vec<RaftMetrics<MemConfig>> {
        let rt = self.nodes.lock().unwrap();
        let mut metrics = vec![];
        for node in rt.values() {
            let m = node.0.metrics().borrow().clone();
            tracing::debug!("router::latest_metrics: {:?}", m);
            metrics.push(m);
        }
        metrics
    }

    pub fn get_metrics(&self, node_id: &MemNodeId) -> anyhow::Result<RaftMetrics<MemConfig>> {
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

    pub fn wait(&self, node_id: &MemNodeId, timeout: Option<Duration>) -> Wait<MemConfig> {
        let node = {
            let rt = self.nodes.lock().unwrap();
            rt.get(node_id).expect("target node not found in routing table").clone().0
        };

        node.wait(timeout)
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
    ) -> Result<ClientWriteResponse<MemConfig>, ClientWriteError<MemConfig>> {
        let node = self.get_raft_handle(&leader).unwrap();
        node.add_learner(target, (), true).await.map_err(|e| e.into_api_error().unwrap())
    }

    /// Ensure read linearizability with user-specified policy.
    pub async fn ensure_linearizable(&self, target: MemNodeId, read_policy: ReadPolicy) -> anyhow::Result<()> {
        let n = self.get_raft_handle(&target)?;
        let linearizer = n.get_read_linearizer(read_policy).await?;
        linearizer.await_ready(&n).await?;
        Ok(())
    }

    /// Get `read_log_id` and last `applied` log
    pub async fn get_read_log_id(
        &self,
        target: MemNodeId,
        read_policy: ReadPolicy,
    ) -> Result<(Option<LogIdOf<MemConfig>>, Option<LogIdOf<MemConfig>>), CheckIsLeaderError<MemConfig>> {
        let n = self.get_raft_handle(&target).unwrap();
        n.get_read_log_id(read_policy).await.map_err(|e| e.into_api_error().unwrap())
    }

    /// Send a client request to the target node, causing test failure on error.
    pub async fn client_request(
        &self,
        mut target: MemNodeId,
        client_id: &str,
        serial: u64,
    ) -> Result<(), RaftError<MemConfig, ClientWriteError<MemConfig>>> {
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
            "Max retry times exceeded. cannot finish client_request, target={}, client_id={} serial={}",
            target, client_id, serial
        )
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

    /// Send multiple client requests to the target node, causing test failure on error.
    /// Returns the number of log written to raft.
    pub async fn client_request_many(
        &self,
        target: MemNodeId,
        client_id: &str,
        count: usize,
    ) -> Result<u64, RaftError<MemConfig, ClientWriteError<MemConfig>>> {
        for idx in 0..count {
            self.client_request(target, client_id, idx as u64).await?;
        }

        Ok(count as u64)
    }

    pub async fn send_client_request(
        &self,
        target: MemNodeId,
        req: ClientRequest,
    ) -> Result<ClientResponse, RaftError<MemConfig, ClientWriteError<MemConfig>>> {
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
        expect_sm_last_applied_log: LogIdOf<TypeConfig>,
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
            vote.leader_id().term(),
            expect_term,
            "expected node {} to have term {}, got {:?}",
            id,
            expect_term,
            vote
        );

        if let Some(voted_for) = &expect_voted_for {
            assert_eq!(
                vote.leader_node_id(),
                Some(voted_for),
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
                &snap.meta.last_log_id.unwrap_or_default().committed_leader_id().term,
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
        expect_sm_last_applied_log: LogIdOf<TypeConfig>,
        expect_snapshot: Option<(ValueTest<u64>, u64)>,
    ) -> anyhow::Result<()> {
        let node_ids = {
            let rt = self.nodes.lock().unwrap();
            rt.keys().cloned().collect::<Vec<_>>()
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

    #[allow(clippy::result_large_err)]
    #[tracing::instrument(level = "debug", skip(self))]
    pub fn emit_rpc_error(&self, id: MemNodeId, target: MemNodeId) -> Result<(), RPCError<MemConfig>> {
        let fails = self.fail_rpc.lock().unwrap();

        for key in [(id, NetSend), (target, NetRecv)] {
            if let Some(err_type) = fails.get(&key) {
                return Err(err_type.make_error(key.0, key.1));
            }
        }

        Ok(())
    }
}

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

impl RaftNetworkV2<MemConfig> for RaftRouterNetwork {
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn append_entries(
        &mut self,
        mut rpc: AppendEntriesRequest<MemConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<MemConfig>, RPCError<MemConfig>> {
        let from_id = rpc.vote.to_leader_node_id().unwrap();

        tracing::debug!("append_entries to id={} {}", self.target, rpc);
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
                        Some(Some(last.log_id()))
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
        let resp = resp.map_err(|e| {
            RPCError::Unreachable(Unreachable::new(&AnyError::error(format!(
                "error: {} target={}",
                e, self.target
            ))))
        })?;

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

    async fn full_snapshot(
        &mut self,
        vote: Vote<MemConfig>,
        snapshot: Snapshot<MemConfig>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<MemConfig>, StreamingError<MemConfig>> {
        let from_id = vote.leader_id().to_node_id().unwrap();

        self.owner.count_rpc(RPCTypes::InstallSnapshot);
        self.owner.call_rpc_pre_hook(snapshot.clone(), from_id, self.target)?;
        self.owner.emit_rpc_error(from_id, self.target)?;
        self.owner.rand_send_delay().await;

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.install_full_snapshot(vote, snapshot).await;
        let resp = resp.map_err(|e| {
            RPCError::Unreachable(Unreachable::new(&AnyError::error(format!(
                "error: {} target={}",
                e, self.target
            ))))
        })?;

        Ok(resp)
    }

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn vote(
        &mut self,
        rpc: VoteRequest<MemConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<MemConfig>, RPCError<MemConfig>> {
        let from_id = rpc.vote.leader_id().to_node_id().unwrap();

        self.owner.count_rpc(RPCTypes::Vote);
        self.owner.call_rpc_pre_hook(rpc.clone(), from_id, self.target)?;
        self.owner.emit_rpc_error(from_id, self.target)?;
        self.owner.rand_send_delay().await;

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.vote(rpc).await;
        let resp = resp.map_err(|e| {
            RPCError::Unreachable(Unreachable::new(&AnyError::error(format!(
                "error: {} target={}",
                e, self.target
            ))))
        })?;

        Ok(resp)
    }

    async fn transfer_leader(
        &mut self,
        rpc: TransferLeaderRequest<MemConfig>,
        _option: RPCOption,
    ) -> Result<(), RPCError<MemConfig>> {
        let from_id = rpc.from_leader().leader_id().to_node_id().unwrap();

        self.owner.count_rpc(RPCTypes::TransferLeader);
        self.owner.call_rpc_pre_hook(rpc.clone(), from_id, self.target)?;
        self.owner.emit_rpc_error(from_id, self.target)?;
        self.owner.rand_send_delay().await;

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.handle_transfer_leader(rpc).await;
        resp.map_err(|e| {
            RPCError::Unreachable(Unreachable::new(&AnyError::error(format!(
                "error: {} target={}",
                e, self.target
            ))))
        })
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

/// A trait for extending the functionality of a Raft vote.
///
/// This is used for testing.
pub(crate) trait TestingVoteExt<C>
where
    C: RaftTypeConfig,
    Self: RaftVote<C>,
{
    /// Creates a new vote for a node in a specific term, with uncommitted status.
    fn from_term_node_id(term: C::Term, node_id: C::NodeId) -> Self {
        let leader_id = C::LeaderId::new(term, node_id);
        Self::from_leader_id(leader_id, false)
    }

    /// Gets the node ID of the leader this vote is for, if present.
    fn to_leader_node_id(&self) -> Option<C::NodeId> {
        self.leader_node_id().cloned()
    }

    /// Gets a reference to the node ID of the leader this vote is for, if present.
    fn leader_node_id(&self) -> Option<&C::NodeId> {
        self.leader_id().and_then(|x| x.node_id())
    }

    // /// Gets the leader ID this vote is associated with.
    // fn to_leader_id(&self) -> C::LeaderId {
    //     self.leader_id().clone()
    // }
    //
    // /// Creates a reference view of this vote.
    // ///
    // /// Returns a lightweight `RefVote` that borrows the data from this vote.
    // fn as_ref_vote(&self) -> RefVote<'_, C> {
    //     RefVote::new(self.leader_id(), self.is_committed())
    // }
    //
    // /// Create a [`CommittedVote`] with the same leader id.
    // fn to_committed(&self) -> CommittedVote<C> {
    //     CommittedVote::new(self.to_leader_id())
    // }
    //
    // /// Create a [`NonCommittedVote`] with the same leader id.
    // fn to_non_committed(&self) -> NonCommittedVote<C> {
    //     NonCommittedVote::new(self.to_leader_id())
    // }
    //
    // /// Convert this vote into a [`CommittedVote`]
    // fn into_committed(self) -> CommittedVote<C> {
    //     CommittedVote::new(self.to_leader_id())
    // }
    //
    // /// Convert this vote into a [`NonCommittedVote`]
    // fn into_non_committed(self) -> NonCommittedVote<C> {
    //     NonCommittedVote::new(self.to_leader_id())
    // }
    //
    // /// Checks if this vote is for the same leader as specified by the given committed leader ID.
    // fn is_same_leader(&self, leader_id: &CommittedLeaderIdOf<C>) -> bool {
    //     self.leader_id().to_committed() == *leader_id
    // }
}

impl<C, T> TestingVoteExt<C> for T
where
    C: RaftTypeConfig,
    T: RaftVote<C>,
{
}
