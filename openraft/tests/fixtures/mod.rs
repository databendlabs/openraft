//! Fixtures for testing Raft.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::env;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Once;
use std::time::Duration;

use anyerror::AnyError;
use anyhow::Context;
use anyhow::Result;
use lazy_static::lazy_static;
use maplit::btreeset;
use memstore::Config as MemConfig;
use memstore::IntoMemClientRequest;
use memstore::MemStore;
use openraft::async_trait::async_trait;
use openraft::error::AddLearnerError;
use openraft::error::AppendEntriesError;
use openraft::error::ClientReadError;
use openraft::error::ClientWriteError;
use openraft::error::InstallSnapshotError;
use openraft::error::NetworkError;
use openraft::error::NodeNotFound;
use openraft::error::RPCError;
use openraft::error::RemoteError;
use openraft::error::VoteError;
use openraft::metrics::Wait;
use openraft::raft::AddLearnerResponse;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::ClientWriteRequest;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::RaftLogReader;
use openraft::storage::RaftStorage;
use openraft::Config;
use openraft::DefensiveCheckBase;
use openraft::LeaderId;
use openraft::LogId;
use openraft::LogIdOptionExt;
use openraft::Node;
use openraft::Raft;
use openraft::RaftMetrics;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::RaftTypeConfig;
use openraft::State;
use openraft::StoreExt;
#[allow(unused_imports)]
use pretty_assertions::assert_eq;
#[allow(unused_imports)]
use pretty_assertions::assert_ne;
use tracing_appender::non_blocking::WorkerGuard;

use crate::fixtures::logging::init_file_logging;

pub mod logging;

#[allow(unused)]
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

#[allow(unused)]
macro_rules! init_ut {
    () => {{
        let name = func_name!();
        let last = name.split("::").last().unwrap();

        let g = crate::fixtures::init_default_ut_tracing();

        let span = tracing::debug_span!("ut", "{}", last);
        (g, span)
    }};
}

pub type StoreWithDefensive<C = MemConfig, S = Arc<MemStore>> = StoreExt<C, S>;

/// A concrete Raft type used during testing.
pub type MemRaft<C = MemConfig, S = Arc<MemStore>> = Raft<C, TypedRaftRouter<C, S>, StoreWithDefensive<C, S>>;

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

/// A type which emulates a network transport and implements the `RaftNetworkFactory` trait.
pub struct TypedRaftRouter<C: RaftTypeConfig = memstore::Config, S: RaftStorage<C> = Arc<MemStore>>
where
    C::D: Debug + IntoMemClientRequest<C::D>,
    C::R: Debug,
    S: Default + Clone,
{
    /// The Raft runtime config which all nodes are using.
    config: Arc<Config>,
    /// The table of all nodes currently known to this router instance.
    #[allow(clippy::type_complexity)]
    routing_table: Arc<Mutex<BTreeMap<C::NodeId, (MemRaft<C, S>, StoreWithDefensive<C, S>)>>>,
    /// Nodes which are isolated can neither send nor receive frames.
    isolated_nodes: Arc<Mutex<HashSet<C::NodeId>>>,

    /// To emulate network delay for sending, in milliseconds.
    /// 0 means no delay.
    send_delay: Arc<AtomicU64>,
}

/// Default `RaftRouter` for memstore.
pub type RaftRouter = TypedRaftRouter<memstore::Config, Arc<MemStore>>;

pub struct Builder<C: RaftTypeConfig, S: RaftStorage<C>> {
    config: Arc<Config>,
    send_delay: u64,
    _phantom: PhantomData<(C, S)>,
}

impl<C: RaftTypeConfig, S: RaftStorage<C>> Builder<C, S>
where
    C::D: Debug + IntoMemClientRequest<C::D>,
    C::R: Debug,
    S: Default + Clone,
{
    pub fn send_delay(mut self, ms: u64) -> Self {
        self.send_delay = ms;
        self
    }

    pub fn build(self) -> TypedRaftRouter<C, S> {
        TypedRaftRouter {
            config: self.config,
            routing_table: Default::default(),
            isolated_nodes: Default::default(),
            send_delay: Arc::new(AtomicU64::new(self.send_delay)),
        }
    }
}

impl<C: RaftTypeConfig, S: RaftStorage<C>> Clone for TypedRaftRouter<C, S>
where
    C::D: Debug + IntoMemClientRequest<C::D>,
    C::R: Debug,
    S: Default + Clone,
{
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            routing_table: self.routing_table.clone(),
            isolated_nodes: self.isolated_nodes.clone(),
            send_delay: self.send_delay.clone(),
        }
    }
}

impl<C: RaftTypeConfig, S: RaftStorage<C>> TypedRaftRouter<C, S>
where
    C::D: Debug + IntoMemClientRequest<C::D>,
    C::R: Debug,
    S: Default + Clone,
{
    pub fn builder(config: Arc<Config>) -> Builder<C, S> {
        Builder {
            config,
            send_delay: 0,
            _phantom: PhantomData,
        }
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

    /// Create a cluster: 0 is the initial leader, others are voters and learners
    /// NOTE: it create a single node cluster first, then change it to a multi-voter cluster.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn new_nodes_from_single(
        &mut self,
        node_ids: BTreeSet<C::NodeId>,
        learners: BTreeSet<C::NodeId>,
    ) -> anyhow::Result<u64> {
        assert!(node_ids.contains(&C::NodeId::default()));

        self.new_raft_node(C::NodeId::default()).await;

        tracing::info!("--- wait for init node to ready");

        self.wait_for_log(&btreeset![C::NodeId::default()], None, timeout(), "empty").await?;
        self.wait_for_state(&btreeset![C::NodeId::default()], State::Learner, timeout(), "empty").await?;

        tracing::info!("--- initializing single node cluster: {}", 0);

        self.initialize_from_single_node(C::NodeId::default()).await?;
        let mut log_index = 1; // log 0: initial membership log; log 1: leader initial log

        tracing::info!("--- wait for init node to become leader");

        self.wait_for_log(&btreeset![C::NodeId::default()], Some(log_index), timeout(), "init").await?;
        self.assert_stable_cluster(Some(1), Some(log_index)).await;

        for id in node_ids.iter() {
            if *id == C::NodeId::default() {
                continue;
            }
            tracing::info!("--- add voter: {}", id);

            self.new_raft_node(*id).await;
            self.add_learner(C::NodeId::default(), *id).await?;
            log_index += 1;
        }
        self.wait_for_log(
            &node_ids,
            Some(log_index),
            timeout(),
            &format!("learners of {:?}", node_ids),
        )
        .await?;

        if node_ids.len() > 1 {
            tracing::info!("--- change membership to setup voters: {:?}", node_ids);

            let node = self.get_raft_handle(&C::NodeId::default())?;
            node.change_membership(node_ids.clone(), true, false).await?;
            log_index += 2;

            self.wait_for_log(
                &node_ids,
                Some(log_index),
                timeout(),
                &format!("cluster of {:?}", node_ids),
            )
            .await?;
        }

        for id in learners.clone() {
            tracing::info!("--- add learner: {}", id);
            self.new_raft_node(id).await;
            self.add_learner(C::NodeId::default(), id).await?;
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
    pub async fn new_raft_node(&mut self, id: C::NodeId) {
        let memstore = self.new_store().await;
        self.new_raft_node_with_sto(id, memstore).await
    }

    pub async fn new_store(&mut self) -> StoreWithDefensive<C, S> {
        let defensive = env::var("RAFT_STORE_DEFENSIVE").ok();

        let sto = StoreExt::<C, S>::new(S::default());

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
    pub async fn new_raft_node_with_sto(&mut self, id: C::NodeId, sto: StoreWithDefensive<C, S>) {
        let node = Raft::new(id, self.config.clone(), self.clone(), sto.clone());
        let mut rt = self.routing_table.lock().unwrap();
        rt.insert(id, (node, sto));
    }

    /// Remove the target node from the routing table & isolation.
    pub async fn remove_node(&mut self, id: C::NodeId) -> Option<(MemRaft<C, S>, StoreWithDefensive<C, S>)> {
        let opt_handles = {
            let mut rt = self.routing_table.lock().unwrap();
            rt.remove(&id)
        };

        {
            let mut isolated = self.isolated_nodes.lock().unwrap();
            isolated.remove(&id);
        }

        opt_handles
    }

    /// Initialize all nodes based on the config in the routing table.
    pub async fn initialize_from_single_node(&self, node_id: C::NodeId) -> Result<()> {
        tracing::info!({ node_id = display(node_id) }, "initializing cluster from single node");
        let members: BTreeSet<C::NodeId> = {
            let rt = self.routing_table.lock().unwrap();
            rt.keys().cloned().collect()
        };

        let node = self.get_raft_handle(&node_id)?;
        node.initialize(members.clone()).await?;
        Ok(())
    }

    /// Initialize cluster with specified node ids.
    pub async fn initialize_with(&self, node_id: C::NodeId, members: BTreeSet<C::NodeId>) -> Result<()> {
        tracing::info!({ node_id = display(node_id) }, "initializing cluster from single node");
        let n = self.get_raft_handle(&node_id)?;
        n.initialize(members.clone()).await?;
        Ok(())
    }

    /// Isolate the network of the specified node.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn isolate_node(&self, id: C::NodeId) {
        self.isolated_nodes.lock().unwrap().insert(id);
    }

    /// Get a payload of the latest metrics from each node in the cluster.
    pub fn latest_metrics(&self) -> Vec<RaftMetrics<C>> {
        let rt = self.routing_table.lock().unwrap();
        let mut metrics = vec![];
        for node in rt.values() {
            metrics.push(node.0.metrics().borrow().clone());
        }
        metrics
    }

    pub fn get_metrics(&self, node_id: &C::NodeId) -> Result<RaftMetrics<C>> {
        let node = self.get_raft_handle(node_id)?;
        let metrics = node.metrics().borrow().clone();
        Ok(metrics)
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub fn get_raft_handle(&self, node_id: &C::NodeId) -> std::result::Result<MemRaft<C, S>, NodeNotFound<C>> {
        let rt = self.routing_table.lock().unwrap();
        let raft_and_sto = rt.get(node_id).ok_or_else(|| NodeNotFound {
            node_id: *node_id,
            source: AnyError::error(""),
        })?;
        let r = raft_and_sto.clone().0;
        Ok(r)
    }

    pub fn get_storage_handle(&self, node_id: &C::NodeId) -> Result<StoreWithDefensive<C, S>> {
        let rt = self.routing_table.lock().unwrap();
        let addr = rt.get(node_id).with_context(|| format!("could not find node {} in routing table", node_id))?;
        let sto = addr.clone().1;
        Ok(sto)
    }

    /// Wait for metrics until it satisfies some condition.
    #[tracing::instrument(level = "info", skip(self, func))]
    pub async fn wait_for_metrics<T>(
        &self,
        node_id: &C::NodeId,
        func: T,
        timeout: Option<Duration>,
        msg: &str,
    ) -> Result<RaftMetrics<C>>
    where
        T: Fn(&RaftMetrics<C>) -> bool + Send,
    {
        let wait = self.wait(node_id, timeout).await?;
        let rst = wait.metrics(func, format!("node-{} {}", node_id, msg)).await?;
        Ok(rst)
    }

    pub async fn wait(&self, node_id: &C::NodeId, timeout: Option<Duration>) -> Result<Wait<C>> {
        let node = {
            let rt = self.routing_table.lock().unwrap();
            rt.get(node_id).expect("target node not found in routing table").clone().0
        };

        Ok(node.wait(timeout))
    }

    /// Wait for specified nodes until they applied upto `want_log`(inclusive) logs.
    #[tracing::instrument(level = "info", skip(self))]
    pub async fn wait_for_log(
        &self,
        node_ids: &BTreeSet<C::NodeId>,
        want_log: Option<u64>,
        timeout: Option<Duration>,
        msg: &str,
    ) -> Result<()> {
        for i in node_ids.iter() {
            self.wait(i, timeout).await?.log(want_log, msg).await?;
        }
        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self))]
    pub async fn wait_for_members(
        &self,
        node_ids: &BTreeSet<C::NodeId>,
        members: BTreeSet<C::NodeId>,
        timeout: Option<Duration>,
        msg: &str,
    ) -> Result<()> {
        for i in node_ids.iter() {
            let wait = self.wait(i, timeout).await?;
            wait.metrics(
                |x| {
                    x.membership_config.get_configs().len() == 1
                        && x.membership_config.get_configs()[0].clone() == members
                },
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
        node_ids: &BTreeSet<C::NodeId>,
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
        node_ids: &BTreeSet<C::NodeId>,
        want: LogId<C>,
        timeout: Option<Duration>,
        msg: &str,
    ) -> Result<()> {
        for i in node_ids.iter() {
            self.wait(i, timeout).await?.snapshot(want, msg).await?;
        }
        Ok(())
    }

    /// Get the ID of the current leader.
    pub fn leader(&self) -> Option<C::NodeId> {
        let isolated = {
            let isolated = self.isolated_nodes.lock().unwrap();
            isolated.clone()
        };

        self.latest_metrics().into_iter().find_map(|node| {
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
    pub async fn restore_node(&self, id: C::NodeId) {
        let mut nodes = self.isolated_nodes.lock().unwrap();
        nodes.remove(&id);
    }

    pub async fn add_learner(
        &self,
        leader: C::NodeId,
        target: C::NodeId,
    ) -> Result<AddLearnerResponse<C>, AddLearnerError<C>> {
        let node = self.get_raft_handle(&leader).unwrap();
        node.add_learner(target, None, true).await
    }

    pub async fn add_learner_with_blocking(
        &self,
        leader: C::NodeId,
        target: C::NodeId,
        blocking: bool,
    ) -> Result<AddLearnerResponse<C>, AddLearnerError<C>> {
        let node = {
            let rt = self.routing_table.lock().unwrap();
            rt.get(&leader).unwrap_or_else(|| panic!("node with ID {} does not exist", leader)).clone()
        };
        node.0.add_learner(target, None, blocking).await
    }

    /// Send a client read request to the target node.
    pub async fn client_read(&self, target: C::NodeId) -> Result<(), ClientReadError<C>> {
        let node = {
            let rt = self.routing_table.lock().unwrap();
            rt.get(&target).unwrap_or_else(|| panic!("node with ID {} does not exist", target)).clone()
        };
        node.0.client_read().await
    }

    /// Send a client request to the target node, causing test failure on error.
    pub async fn client_request(&self, target: C::NodeId, client_id: &str, serial: u64) {
        let req = <C::D as IntoMemClientRequest<C::D>>::make_request(client_id, serial);
        if let Err(err) = self.send_client_request(target, req).await {
            tracing::error!({error=%err}, "error from client request");
            panic!("{:?}", err)
        }
    }

    /// Request the current leader from the target node.
    pub async fn current_leader(&self, target: C::NodeId) -> Option<C::NodeId> {
        let node = self.get_raft_handle(&target).unwrap();
        node.current_leader().await
    }

    /// Send multiple client requests to the target node, causing test failure on error.
    pub async fn client_request_many(&self, target: C::NodeId, client_id: &str, count: usize) {
        for idx in 0..count {
            self.client_request(target, client_id, idx as u64).await
        }
    }

    async fn send_client_request(
        &self,
        target: C::NodeId,
        req: C::D,
    ) -> std::result::Result<C::R, ClientWriteError<C>> {
        let node = {
            let rt = self.routing_table.lock().unwrap();
            rt.get(&target)
                .unwrap_or_else(|| panic!("node '{}' does not exist in routing table", target))
                .clone()
        };

        let payload = EntryPayload::<C>::Normal(req);

        node.0.client_write(ClientWriteRequest::<C>::new(payload)).await.map(|res| res.data)
    }

    /// Assert that the cluster is in a pristine state, with all nodes as learners.
    pub async fn assert_pristine_cluster(&self) {
        let nodes = self.latest_metrics();
        for node in nodes.iter() {
            assert!(
                node.current_leader.is_none(),
                "node {} has a current leader: {:?}, expected none",
                node.id,
                node.current_leader,
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
                None,
                node.last_applied.index(),
                "node {} has last_applied {:?}, expected None",
                node.id,
                node.last_applied
            );
            assert_eq!(
                None, node.last_log_index,
                "node {} has last_log_index {:?}, expected None",
                node.id, node.last_log_index
            );
            let members = node.membership_config.get_configs()[0].clone();
            assert_eq!(
                members.iter().cloned().collect::<Vec<_>>(),
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
        let isolated = {
            let x = self.isolated_nodes.lock().unwrap();
            x.clone()
        };
        let nodes = self.latest_metrics();

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
        let expected_last_log = if expected_last_log.is_some() {
            expected_last_log
        } else {
            leader.last_log_index
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
                node.last_applied.index(),
                expected_last_log,
                "node {} has last_applied {:?}, expected {:?}",
                node.id,
                node.last_applied,
                expected_last_log
            );
            assert_eq!(
                node.last_log_index, expected_last_log,
                "node {} has last_log_index {:?}, expected {:?}",
                node.id, node.last_log_index, expected_last_log
            );
            let members = node.membership_config.get_configs()[0].clone();
            let mut members = members.into_iter().collect::<Vec<_>>();
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

    /// Assert against the state of the storage system one node in the cluster.
    pub async fn assert_storage_state_with_sto(
        &self,
        storage: &mut StoreWithDefensive<C, S>,
        id: &C::NodeId,
        expect_term: u64,
        expect_last_log: u64,
        expect_voted_for: Option<C::NodeId>,
        expect_sm_last_applied_log: LogId<C>,
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
            vote.term, expect_term,
            "expected node {} to have term {}, got {:?}",
            id, expect_term, vote
        );

        if let Some(voted_for) = &expect_voted_for {
            assert_eq!(
                vote.node_id, *voted_for,
                "expected node {} to have voted for {}, got {:?}",
                id, voted_for, vote
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
                &snap.meta.last_log_id.leader_id.term, term,
                "expected node {} to have snapshot with term {}, got {}",
                id, term, snap.meta.last_log_id.leader_id.term
            );
        }

        let (last_applied, _) = storage.last_applied_state().await?;

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

    /// Assert against the state of the storage system one node in the cluster.
    pub async fn assert_storage_state_in_node(
        &self,
        node_id: C::NodeId,
        expect_term: u64,
        expect_last_log: u64,
        expect_voted_for: Option<C::NodeId>,
        expect_sm_last_applied_log: LogId<C>,
        expect_snapshot: Option<(ValueTest<u64>, u64)>,
    ) -> anyhow::Result<()> {
        let mut rt = self.routing_table.lock().unwrap();

        for (id, (_node, storage)) in rt.iter_mut() {
            if *id != node_id {
                continue;
            }
            self.assert_storage_state_with_sto(
                storage,
                id,
                expect_term,
                expect_last_log,
                expect_voted_for,
                expect_sm_last_applied_log,
                &expect_snapshot,
            )
            .await?;

            break;
        }

        Ok(())
    }

    /// Assert against the state of the storage system per node in the cluster.
    pub async fn assert_storage_state(
        &self,
        expect_term: u64,
        expect_last_log: u64,
        expect_voted_for: Option<C::NodeId>,
        expect_sm_last_applied_log: LogId<C>,
        expect_snapshot: Option<(ValueTest<u64>, u64)>,
    ) -> anyhow::Result<()> {
        let mut rt = self.routing_table.lock().unwrap();

        for (id, (_node, storage)) in rt.iter_mut() {
            self.assert_storage_state_with_sto(
                storage,
                id,
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
    pub fn check_reachable(&self, id: C::NodeId, target: C::NodeId) -> std::result::Result<(), NetworkError> {
        let isolated = self.isolated_nodes.lock().unwrap();

        if isolated.contains(&target) || isolated.contains(&id) {
            let network_err = NetworkError::new(&AnyError::error(format!("isolated:{} -> {}", id, target)));
            return Err(network_err);
        }

        Ok(())
    }
}

#[async_trait]
impl<C: RaftTypeConfig, S: RaftStorage<C>> RaftNetworkFactory<C> for TypedRaftRouter<C, S>
where
    C::D: Debug + IntoMemClientRequest<C::D>,
    C::R: Debug,
    S: Default + Clone,
{
    type Network = RaftRouterNetwork<C, S>;

    async fn connect(&mut self, target: C::NodeId, _node: Option<&Node>) -> Self::Network {
        RaftRouterNetwork {
            target,
            owner: self.clone(),
        }
    }
}

pub struct RaftRouterNetwork<C: RaftTypeConfig, S: RaftStorage<C>>
where
    C::D: Debug + IntoMemClientRequest<C::D>,
    C::R: Debug,
    S: Default + Clone,
{
    target: C::NodeId,
    owner: TypedRaftRouter<C, S>,
}

#[async_trait]
impl<C: RaftTypeConfig, S: RaftStorage<C>> RaftNetwork<C> for RaftRouterNetwork<C, S>
where
    C::D: Debug + IntoMemClientRequest<C::D>,
    C::R: Debug,
    S: Default + Clone,
{
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn send_append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
    ) -> std::result::Result<AppendEntriesResponse<C>, RPCError<C, AppendEntriesError<C>>> {
        tracing::debug!("append_entries to id={} {:?}", self.target, rpc);
        self.owner.rand_send_delay().await;

        self.owner.check_reachable(rpc.vote.node_id, self.target)?;

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.append_entries(rpc).await;

        tracing::debug!("append_entries: recv resp from id={} {:?}", self.target, resp);
        let resp = resp.map_err(|e| RemoteError::new(self.target, e))?;
        Ok(resp)
    }

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    async fn send_install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<C>,
    ) -> std::result::Result<InstallSnapshotResponse<C>, RPCError<C, InstallSnapshotError<C>>> {
        self.owner.rand_send_delay().await;

        self.owner.check_reachable(rpc.vote.node_id, self.target)?;

        let node = self.owner.get_raft_handle(&self.target)?;

        let resp = node.install_snapshot(rpc).await;
        let resp = resp.map_err(|e| RemoteError::new(self.target, e))?;
        Ok(resp)
    }

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn send_vote(
        &mut self,
        rpc: VoteRequest<C>,
    ) -> std::result::Result<VoteResponse<C>, RPCError<C, VoteError<C>>> {
        self.owner.rand_send_delay().await;

        self.owner.check_reachable(rpc.vote.node_id, self.target)?;

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
    Some(Duration::from_millis(5000))
}

/// Create a blank log entry for test.
pub fn blank<C: RaftTypeConfig>(term: u64, index: u64) -> Entry<C>
where C::NodeId: From<u64> {
    Entry {
        log_id: LogId::new(LeaderId::new(term, 0.into()), index),
        payload: EntryPayload::Blank,
    }
}
