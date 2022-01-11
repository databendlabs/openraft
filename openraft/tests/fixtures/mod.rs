//! Fixtures for testing Raft.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::env;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Once;
use std::time::Duration;

use anyhow::anyhow;
use anyhow::Context;
use anyhow::Result;
use lazy_static::lazy_static;
use maplit::btreeset;
use memstore::ClientRequest as MemClientRequest;
use memstore::ClientRequest;
use memstore::ClientResponse;
use memstore::ClientResponse as MemClientResponse;
use memstore::MemStore;
use openraft::async_trait::async_trait;
use openraft::error::AddLearnerError;
use openraft::error::ClientReadError;
use openraft::error::ClientWriteError;
use openraft::metrics::Wait;
use openraft::raft::AddLearnerResponse;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::ClientWriteRequest;
use openraft::raft::ClientWriteResponse;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::RaftStorage;
use openraft::AppData;
use openraft::Config;
use openraft::DefensiveCheck;
use openraft::LogId;
use openraft::NodeId;
use openraft::Raft;
use openraft::RaftMetrics;
use openraft::RaftNetwork;
use openraft::State;
use openraft::StoreExt;
#[allow(unused_imports)]
use pretty_assertions::assert_eq;
#[allow(unused_imports)]
use pretty_assertions::assert_ne;
use tokio::sync::RwLock;
use tracing_appender::non_blocking::WorkerGuard;

use crate::fixtures::logging::init_file_logging;

pub mod logging;

macro_rules! func_name {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let name = type_name_of(f);
        let n = &name[..name.len() - 3];
        let nn = n.replace("::{{closure}}", "");
        nn
    }};
}

macro_rules! init_ut {
    () => {{
        let name = func_name!();
        let last = name.split("::").last().unwrap();

        let g = crate::fixtures::init_default_ut_tracing();

        let span = tracing::debug_span!("ut", "{}", last);
        (g, span)
    }};
}

pub type StoreWithDefensive = StoreExt<ClientRequest, ClientResponse, MemStore>;

/// A concrete Raft type used during testing.
pub type MemRaft = Raft<MemClientRequest, MemClientResponse, RaftRouter, StoreWithDefensive>;

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
    let (g, sub) = init_file_logging(app_name, dir, level);
    tracing::subscriber::set_global_default(sub).expect("error setting global tracing subscriber");

    tracing::info!("initialized global tracing: in {}/{} at {}", dir, app_name, level);
    g
}

/// A type which emulates a network transport and implements the `RaftNetwork` trait.
pub struct RaftRouter {
    /// The Raft runtime config which all nodes are using.
    config: Arc<Config>,
    /// The table of all nodes currently known to this router instance.
    routing_table: RwLock<BTreeMap<NodeId, (MemRaft, Arc<StoreWithDefensive>)>>,
    /// Nodes which are isolated can neither send nor receive frames.
    isolated_nodes: RwLock<HashSet<NodeId>>,

    /// To enumlate network delay for sending, in milli second.
    /// 0 means no delay.
    send_delay: u64,
}

pub struct Builder {
    config: Arc<Config>,
    send_delay: u64,
}

impl Builder {
    pub fn send_delay(mut self, ms: u64) -> Self {
        self.send_delay = ms;
        self
    }

    pub fn build(self) -> RaftRouter {
        RaftRouter {
            config: self.config,
            routing_table: Default::default(),
            isolated_nodes: Default::default(),
            send_delay: self.send_delay,
        }
    }
}

impl RaftRouter {
    pub fn builder(config: Arc<Config>) -> Builder {
        Builder { config, send_delay: 0 }
    }

    /// Create a new instance.
    pub fn new(config: Arc<Config>) -> Self {
        Self::builder(config).build()
    }

    pub fn network_send_delay(&mut self, ms: u64) {
        self.send_delay = ms;
    }

    async fn rand_send_delay(&self) {
        if self.send_delay == 0 {
            return;
        }

        let r = rand::random::<u64>() % self.send_delay;
        let timeout = Duration::from_millis(r);
        tokio::time::sleep(timeout).await;
    }

    /// Create a cluster: 0 is the initial leader, others are voters learners
    /// NOTE: it create a single node cluster first, then change it to a multi-voter cluster.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn new_nodes_from_single(
        self: &Arc<Self>,
        node_ids: BTreeSet<NodeId>,
        learners: BTreeSet<NodeId>,
    ) -> anyhow::Result<u64> {
        assert!(node_ids.contains(&0));

        self.new_raft_node(0).await;

        let mut n_logs = 0;

        tracing::info!("--- wait for init node to ready");

        self.wait_for_log(&btreeset![0], n_logs, timeout(), "empty").await?;
        self.wait_for_state(&btreeset![0], State::Learner, timeout(), "empty").await?;

        tracing::info!("--- initializing single node cluster: {}", 0);

        self.initialize_from_single_node(0).await?;
        n_logs += 1;

        tracing::info!("--- wait for init node to become leader");

        self.wait_for_log(&btreeset![0], n_logs, timeout(), "init").await?;
        self.assert_stable_cluster(Some(1), Some(n_logs)).await;

        for id in node_ids.iter() {
            if *id == 0 {
                continue;
            }
            tracing::info!("--- add voter: {}", id);

            self.new_raft_node(*id).await;
            self.add_learner(0, *id).await?;
        }

        if node_ids.len() > 1 {
            tracing::info!("--- change membership to setup voters: {:?}", node_ids);

            self.change_membership(0, node_ids.clone()).await?;
            n_logs += 2;

            self.wait_for_log(&node_ids, n_logs, timeout(), &format!("cluster of {:?}", node_ids)).await?;
        }

        for id in learners {
            tracing::info!("--- add learner: {}", id);
            self.new_raft_node(id).await;
            self.add_learner(0, id).await?;
        }

        Ok(n_logs)
    }

    /// Create and register a new Raft node bearing the given ID.
    pub async fn new_raft_node(self: &Arc<Self>, id: NodeId) {
        let memstore = self.new_store(id).await;
        self.new_raft_node_with_sto(id, memstore).await
    }

    pub async fn new_store(self: &Arc<Self>, id: u64) -> Arc<StoreWithDefensive> {
        let defensive = env::var("RAFT_STORE_DEFENSIVE").ok();

        let sto = Arc::new(StoreExt::new(MemStore::new(id).await));

        if let Some(d) = defensive {
            tracing::info!("RAFT_STORE_DEFENSIVE set store defensive to {}", d);

            let want = if d == "on" {
                true
            } else if d == "off" {
                false
            } else {
                tracing::warn!("unknown value of RAFT_STORE_DEFENSIVE: {}", d);
                return sto;
            };

            sto.set_defensive(want);
            if sto.is_defensive() != want {
                tracing::error!("failure to set store defensive to {}", want);
            }
        }

        sto
    }

    #[tracing::instrument(level = "debug", skip(self, sto))]
    pub async fn new_raft_node_with_sto(self: &Arc<Self>, id: NodeId, sto: Arc<StoreWithDefensive>) {
        let node = Raft::new(id, self.config.clone(), self.clone(), sto.clone());
        let mut rt = self.routing_table.write().await;
        rt.insert(id, (node, sto));
    }

    /// Remove the target node from the routing table & isolation.
    pub async fn remove_node(&self, id: NodeId) -> Option<(MemRaft, Arc<StoreWithDefensive>)> {
        let mut rt = self.routing_table.write().await;
        let opt_handles = rt.remove(&id);
        let mut isolated = self.isolated_nodes.write().await;
        isolated.remove(&id);

        opt_handles
    }

    /// Initialize all nodes based on the config in the routing table.
    pub async fn initialize_from_single_node(&self, node: NodeId) -> Result<()> {
        tracing::info!({ node }, "initializing cluster from single node");
        let rt = self.routing_table.read().await;
        let members: BTreeSet<NodeId> = rt.keys().cloned().collect();
        rt.get(&node)
            .ok_or_else(|| anyhow!("node {} not found in routing table", node))?
            .0
            .initialize(members.clone())
            .await?;
        Ok(())
    }

    /// Initialize cluster with specified node ids.
    pub async fn initialize_with(&self, node: NodeId, members: BTreeSet<NodeId>) -> Result<()> {
        tracing::info!({ node }, "initializing cluster from single node");
        let rt = self.routing_table.read().await;
        rt.get(&node)
            .ok_or_else(|| anyhow!("node {} not found in routing table", node))?
            .0
            .initialize(members.clone())
            .await?;
        Ok(())
    }

    /// Isolate the network of the specified node.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn isolate_node(&self, id: NodeId) {
        self.isolated_nodes.write().await.insert(id);
    }

    /// Get a payload of the latest metrics from each node in the cluster.
    pub async fn latest_metrics(&self) -> Vec<RaftMetrics> {
        let rt = self.routing_table.read().await;
        let mut metrics = vec![];
        for node in rt.values() {
            metrics.push(node.0.metrics().borrow().clone());
        }
        metrics
    }

    pub async fn get_metrics(&self, node_id: &NodeId) -> Result<RaftMetrics> {
        let rt = self.routing_table.read().await;
        let x = rt.get(node_id).with_context(|| format!("could not find node {} in routing table", node_id))?;
        let metrics = x.0.metrics().borrow().clone();
        Ok(metrics)
    }

    /// Get a handle to the storage backend for the target node.
    pub async fn get_storage_handle(&self, node_id: &NodeId) -> Result<Arc<StoreWithDefensive>> {
        let rt = self.routing_table.read().await;
        let addr = rt.get(node_id).with_context(|| format!("could not find node {} in routing table", node_id))?;
        let sto = addr.clone().1;
        Ok(sto)
    }

    /// Wait for metrics until it satisfies some condition.
    #[tracing::instrument(level = "info", skip(self, func))]
    pub async fn wait_for_metrics<T>(
        &self,
        node_id: &NodeId,
        func: T,
        timeout: Option<Duration>,
        msg: &str,
    ) -> Result<RaftMetrics>
    where
        T: Fn(&RaftMetrics) -> bool + Send,
    {
        let wait = self.wait(node_id, timeout).await?;
        let rst = wait.metrics(func, format!("node-{} {}", node_id, msg)).await?;
        Ok(rst)
    }

    pub async fn wait(&self, node_id: &NodeId, timeout: Option<Duration>) -> Result<Wait> {
        let rt = self.routing_table.read().await;
        let node = rt.get(node_id).with_context(|| format!("node {} not found", node_id))?;

        Ok(node.0.wait(timeout))
    }

    /// Wait for specified nodes until they applied upto `want_log`(inclusive) logs.
    #[tracing::instrument(level = "info", skip(self))]
    pub async fn wait_for_log(
        &self,
        node_ids: &BTreeSet<u64>,
        want_log: u64,
        timeout: Option<Duration>,
        msg: &str,
    ) -> Result<()> {
        for i in node_ids.iter() {
            self.wait(i, timeout).await?.log(want_log, msg).await?;
        }
        Ok(())
    }

    /// Wait for specified nodes until their state becomes `state`.
    #[tracing::instrument(level = "info", skip(self))]
    pub async fn wait_for_state(
        &self,
        node_ids: &BTreeSet<u64>,
        want_state: State,
        timeout: Option<Duration>,
        msg: &str,
    ) -> Result<()> {
        for i in node_ids.iter() {
            self.wait(i, timeout).await?.state(want_state, msg).await?;
        }
        Ok(())
    }

    /// Wait for specified nodes until their snapshot becomes `want`.
    #[tracing::instrument(level = "info", skip(self))]
    pub async fn wait_for_snapshot(
        &self,
        node_ids: &BTreeSet<u64>,
        want: LogId,
        timeout: Option<Duration>,
        msg: &str,
    ) -> Result<()> {
        for i in node_ids.iter() {
            self.wait(i, timeout).await?.snapshot(want, msg).await?;
        }
        Ok(())
    }

    /// Get the ID of the current leader.
    pub async fn leader(&self) -> Option<NodeId> {
        let isolated = self.isolated_nodes.read().await;
        self.latest_metrics().await.into_iter().find_map(|node| {
            if node.current_leader == Some(node.id) {
                if isolated.contains(&node.id) {
                    None
                } else {
                    Some(node.id)
                }
            } else {
                None
            }
        })
    }

    /// Restore the network of the specified node.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn restore_node(&self, id: NodeId) {
        let mut nodes = self.isolated_nodes.write().await;
        nodes.remove(&id);
    }

    pub async fn add_learner(&self, leader: NodeId, target: NodeId) -> Result<AddLearnerResponse, AddLearnerError> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&leader).unwrap_or_else(|| panic!("node with ID {} does not exist", leader));
        node.0.add_learner(target, true).await
    }

    pub async fn add_learner_with_blocking(
        &self,
        leader: NodeId,
        target: NodeId,
        blocking: bool,
    ) -> Result<AddLearnerResponse, AddLearnerError> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&leader).unwrap_or_else(|| panic!("node with ID {} does not exist", leader));
        node.0.add_learner(target, blocking).await
    }

    pub async fn change_membership(
        &self,
        leader: NodeId,
        members: BTreeSet<NodeId>,
    ) -> Result<ClientWriteResponse<MemClientResponse>, ClientWriteError> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&leader).unwrap_or_else(|| panic!("node with ID {} does not exist", leader));
        node.0.change_membership(members, true).await
    }

    pub async fn change_membership_with_blocking(
        &self,
        leader: NodeId,
        members: BTreeSet<NodeId>,
        blocking: bool,
    ) -> Result<ClientWriteResponse<MemClientResponse>, ClientWriteError> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&leader).unwrap_or_else(|| panic!("node with ID {} does not exist", leader));
        node.0.change_membership(members, blocking).await
    }

    /// Send a client read request to the target node.
    pub async fn client_read(&self, target: NodeId) -> Result<(), ClientReadError> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&target).unwrap_or_else(|| panic!("node with ID {} does not exist", target));
        node.0.client_read().await
    }

    /// Send a client request to the target node, causing test failure on error.
    pub async fn client_request(&self, target: NodeId, client_id: &str, serial: u64) {
        let req = MemClientRequest {
            client: client_id.into(),
            serial,
            status: format!("request-{}", serial),
        };
        if let Err(err) = self.send_client_request(target, req).await {
            tracing::error!({error=%err}, "error from client request");
            panic!("{:?}", err)
        }
    }

    /// Request the current leader from the target node.
    pub async fn current_leader(&self, target: NodeId) -> Option<NodeId> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&target).unwrap_or_else(|| panic!("node with ID {} does not exist", target));
        node.0.current_leader().await
    }

    /// Send multiple client requests to the target node, causing test failure on error.
    pub async fn client_request_many(&self, target: NodeId, client_id: &str, count: usize) {
        for idx in 0..count {
            self.client_request(target, client_id, idx as u64).await
        }
    }

    async fn send_client_request(
        &self,
        target: NodeId,
        req: MemClientRequest,
    ) -> std::result::Result<MemClientResponse, ClientWriteError> {
        let rt = self.routing_table.read().await;
        let node = rt.get(&target).unwrap_or_else(|| panic!("node '{}' does not exist in routing table", target));
        node.0.client_write(ClientWriteRequest::new(req)).await.map(|res| res.data)
    }

    //////////////////////////////////////////////////////////////////////////////////////////////

    /// Assert that the cluster is in a pristine state, with all nodes as learners.
    pub async fn assert_pristine_cluster(&self) {
        let nodes = self.latest_metrics().await;
        for node in nodes.iter() {
            assert!(
                node.current_leader.is_none(),
                "node {} has a current leader, expected none",
                node.id
            );
            assert_eq!(
                node.state,
                State::Learner,
                "node is in state {:?}, expected Learner",
                node.state
            );
            assert_eq!(
                node.current_term, 0,
                "node {} has term {}, expected 0",
                node.id, node.current_term
            );
            assert_eq!(
                node.last_applied, 0,
                "node {} has last_applied {}, expected 0",
                node.id, node.last_applied
            );
            assert_eq!(
                node.last_log_index, 0,
                "node {} has last_log_index {}, expected 0",
                node.id, node.last_log_index
            );
            let members = node.membership_config.membership.ith_config(0);
            assert_eq!(
                members,
                vec![node.id],
                "node {0} has membership {1:?}, expected [{0}]",
                node.id,
                members
            );
            assert!(
                !node.membership_config.membership.is_in_joint_consensus(),
                "node {} is in joint consensus, expected uniform consensus",
                node.id
            );
        }
    }

    /// Assert that the cluster has an elected leader, and is in a stable state with all nodes uniform.
    ///
    /// If `expected_term` is `Some`, then all nodes will be tested to ensure that they are in the
    /// given term. Else, the leader's current term will be used for the assertion.
    ///
    /// If `expected_last_log` is `Some`, then all nodes will be tested to ensure that their last
    /// log index and last applied log match the given value. Else, the leader's last_log_index
    /// will be used for the assertion.
    pub async fn assert_stable_cluster(&self, expected_term: Option<u64>, expected_last_log: Option<u64>) {
        let isolated = self.isolated_nodes.read().await;
        let nodes = self.latest_metrics().await;

        let non_isolated_nodes: Vec<_> = nodes.iter().filter(|node| !isolated.contains(&node.id)).collect();
        let leader = nodes
            .iter()
            .filter(|node| !isolated.contains(&node.id))
            .find(|node| node.state == State::Leader)
            .expect("expected to find a cluster leader");
        let followers: Vec<_> = nodes
            .iter()
            .filter(|node| !isolated.contains(&node.id))
            .filter(|node| node.state == State::Follower)
            .collect();

        assert_eq!(
            followers.len() + 1,
            non_isolated_nodes.len(),
            "expected all nodes to be followers with one leader, got 1 leader and {} followers, expected {} followers",
            followers.len(),
            non_isolated_nodes.len() - 1,
        );
        let expected_term = match expected_term {
            Some(term) => term,
            None => leader.current_term,
        };
        let expected_last_log = match expected_last_log {
            Some(idx) => idx,
            None => leader.last_log_index,
        };
        let all_nodes = nodes.iter().map(|node| node.id).collect::<Vec<_>>();
        for node in non_isolated_nodes.iter() {
            assert_eq!(
                node.current_leader,
                Some(leader.id),
                "node {} has leader {:?}, expected {}",
                node.id,
                node.current_leader,
                leader.id
            );
            assert_eq!(
                node.current_term, expected_term,
                "node {} has term {}, expected {}",
                node.id, node.current_term, expected_term
            );
            assert_eq!(
                node.last_applied, expected_last_log,
                "node {} has last_applied {}, expected {}",
                node.id, node.last_applied, expected_last_log
            );
            assert_eq!(
                node.last_log_index, expected_last_log,
                "node {} has last_log_index {}, expected {}",
                node.id, node.last_log_index, expected_last_log
            );
            let mut members = node.membership_config.membership.ith_config(0);
            members.sort_unstable();
            assert_eq!(
                members, all_nodes,
                "node {} has membership {:?}, expected {:?}",
                node.id, members, all_nodes
            );
            assert!(
                !node.membership_config.membership.is_in_joint_consensus(),
                "node {} was not in uniform consensus state",
                node.id
            );
        }
    }

    /// Assert against the state of the storage system per node in the cluster.
    pub async fn assert_storage_state(
        &self,
        expect_term: u64,
        expect_last_log: u64,
        expect_voted_for: Option<u64>,
        expect_sm_last_applied_log: LogId,
        expect_snapshot: Option<(ValueTest<u64>, u64)>,
    ) -> anyhow::Result<()> {
        let rt = self.routing_table.read().await;
        for (id, (_node, storage)) in rt.iter() {
            let (sm_last_id, _) = storage.last_applied_state().await?;
            let last_log_id = match storage.last_id_in_log().await? {
                Some(log_last_id) => std::cmp::max(log_last_id, sm_last_id),
                None => sm_last_id,
            };

            assert_eq!(
                expect_last_log, last_log_id.index,
                "expected node {} to have last_log {}, got {}",
                id, expect_last_log, last_log_id
            );

            let hs = storage.read_hard_state().await?.unwrap_or_else(|| panic!("no hard state found for node {}", id));

            assert_eq!(
                hs.current_term, expect_term,
                "expected node {} to have term {}, got {:?}",
                id, expect_term, hs
            );

            if let Some(voted_for) = &expect_voted_for {
                assert_eq!(
                    hs.voted_for.as_ref(),
                    Some(voted_for),
                    "expected node {} to have voted for {}, got {:?}",
                    id,
                    voted_for,
                    hs
                );
            }

            if let Some((index_test, term)) = &expect_snapshot {
                let snap = storage
                    .get_current_snapshot()
                    .await
                    .map_err(|err| panic!("{}", err))
                    .unwrap()
                    .unwrap_or_else(|| panic!("no snapshot present for node {}", id));

                match index_test {
                    ValueTest::Exact(index) => assert_eq!(
                        &snap.meta.last_log_id.index, index,
                        "expected node {} to have snapshot with index {}, got {}",
                        id, index, snap.meta.last_log_id.index
                    ),
                    ValueTest::Range(range) => assert!(
                        range.contains(&snap.meta.last_log_id.index),
                        "expected node {} to have snapshot within range {:?}, got {}",
                        id,
                        range,
                        snap.meta.last_log_id.index
                    ),
                }

                assert_eq!(
                    &snap.meta.last_log_id.term, term,
                    "expected node {} to have snapshot with term {}, got {}",
                    id, term, snap.meta.last_log_id.term
                );
            }

            let (last_applied, _) = storage.last_applied_state().await?;

            assert_eq!(
                &last_applied, &expect_sm_last_applied_log,
                "expected node {} to have state machine last_applied_log {}, got {}",
                id, expect_sm_last_applied_log, last_applied
            );
        }

        Ok(())
    }
}

#[async_trait]
impl RaftNetwork<MemClientRequest> for RaftRouter {
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn send_append_entries(
        &self,
        target: u64,
        rpc: AppendEntriesRequest<MemClientRequest>,
    ) -> Result<AppendEntriesResponse> {
        tracing::debug!("append_entries to id={} {:?}", target, rpc);
        self.rand_send_delay().await;

        let rt = self.routing_table.read().await;
        let isolated = self.isolated_nodes.read().await;
        let addr = rt.get(&target).expect("target node not found in routing table");
        if isolated.contains(&target) || isolated.contains(&rpc.leader_id) {
            return Err(anyhow!("target node is isolated"));
        }
        let resp = addr.0.append_entries(rpc).await;

        tracing::debug!("append_entries: recv resp from id={} {:?}", target, resp);
        Ok(resp?)
    }

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    async fn send_install_snapshot(&self, target: u64, rpc: InstallSnapshotRequest) -> Result<InstallSnapshotResponse> {
        self.rand_send_delay().await;

        let rt = self.routing_table.read().await;
        let isolated = self.isolated_nodes.read().await;
        let addr = rt.get(&target).expect("target node not found in routing table");
        if isolated.contains(&target) || isolated.contains(&rpc.leader_id) {
            return Err(anyhow!("target node is isolated"));
        }
        Ok(addr.0.install_snapshot(rpc).await?)
    }

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn send_vote(&self, target: u64, rpc: VoteRequest) -> Result<VoteResponse> {
        self.rand_send_delay().await;

        let rt = self.routing_table.read().await;
        let isolated = self.isolated_nodes.read().await;
        let addr = rt.get(&target).expect("target node not found in routing table");
        if isolated.contains(&target) || isolated.contains(&rpc.candidate_id) {
            return Err(anyhow!("target node is isolated"));
        }
        Ok(addr.0.vote(rpc).await?)
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
    Some(Duration::from_millis(5000))
}

/// Create a blank log entry for test.
pub fn ent<T: AppData>(term: u64, index: u64) -> Entry<T> {
    Entry {
        log_id: LogId { term, index },
        payload: EntryPayload::Blank,
    }
}
