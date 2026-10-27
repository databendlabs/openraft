use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use anyhow::anyhow;
use openraft::Config;
use openraft::Raft;
use openraft::RaftTypeConfig;
use openraft::SnapshotPolicy;
use openraft::alias::EntryOf;
use openraft::entry::RaftEntry;
use openraft::errors::RPCError;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::RaftLogStorageExt;
use openraft::testing::log_id;

use crate::fixtures::custom_membership::log_store::LogStore;
use crate::fixtures::custom_membership::network::Network;
use crate::fixtures::custom_membership::network::Router;
use crate::fixtures::custom_membership::state_machine::StateMachine;
use crate::fixtures::harness::record_network_delay;
use crate::fixtures::network_delay::configured_network_send_delay;

/// A type config that [`TestCluster`] can run.
pub trait TestConfig
where Self: RaftTypeConfig<NodeId = u64, Node = (), Term = u64, R = ()>
{
}

impl<C> TestConfig for C where C: RaftTypeConfig<NodeId = u64, Node = (), Term = u64, R = ()> {}

pub type TestRaft<C> = Raft<C, StateMachine<C>>;

/// A config that leaves the cluster idle until the test campaigns.
pub fn cluster_config(snapshot_policy: SnapshotPolicy) -> Config {
    Config {
        enable_tick: false,
        snapshot_policy,
        ..Default::default()
    }
}

/// Nodes of one membership, connected through one router.
pub struct TestCluster<C>
where C: TestConfig
{
    config: Arc<Config>,
    router: Arc<Mutex<Router<C>>>,
    log_stores: BTreeMap<u64, LogStore<C>>,
    state_machines: BTreeMap<u64, StateMachine<C>>,
}

impl<C> TestCluster<C>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    /// Starts one node per id in `node_ids` with `config`.
    ///
    /// Each node holds only `membership`, at log index 0, and no vote, so no node leads until the
    /// test triggers an election. `config` is validated and kept as given.
    pub async fn start(
        config: Config,
        node_ids: impl IntoIterator<Item = u64>,
        membership: C::Membership,
    ) -> Result<Self> {
        let config = config.validate()?;
        let config = Arc::new(config);
        let delay_ms = configured_network_send_delay(0);
        let send_delay = Arc::new(AtomicU64::new(delay_ms));
        record_network_delay(&send_delay);
        let router = Arc::new(Mutex::new(Router {
            nodes: BTreeMap::new(),
            cut_links: BTreeSet::new(),
            send_delay,
        }));
        let mut log_stores = BTreeMap::new();
        let mut state_machines = BTreeMap::new();

        for node_id in node_ids {
            let mut log_store = LogStore::<C>::default();
            let initial_log_id = log_id::<C>(0, 0, 0);
            let membership_entry = EntryOf::<C>::new_membership(initial_log_id, membership.clone());
            log_store.blocking_append([membership_entry]).await?;
            log_stores.insert(node_id, log_store.clone());

            let state_machine = StateMachine::<C>::default();
            let network = Network {
                source: node_id,
                router: router.clone(),
            };
            let raft = Raft::new(node_id, config.clone(), network, log_store, state_machine.clone()).await?;

            let mut locked_router = router.lock().unwrap();
            locked_router.nodes.insert(node_id, raft);
            drop(locked_router);

            state_machines.insert(node_id, state_machine);
        }

        Ok(Self {
            config,
            router,
            log_stores,
            state_machines,
        })
    }

    pub fn raft(&self, node_id: u64) -> TestRaft<C> {
        let router = self.router.lock().unwrap();
        router.nodes[&node_id].clone()
    }

    /// Set the delay before each RPC, in milliseconds.
    ///
    /// The value replaces the delay read from `OPENRAFT_NETWORK_SEND_DELAY` at start.
    pub fn network_send_delay(&self, ms: u64) {
        tracing::info!(send_delay_ms = ms, "router network delay changed");
        let router = self.router.lock().unwrap();
        router.send_delay.store(ms, Ordering::Relaxed);
    }

    /// Deliver one vote RPC from `source` to `target` through this cluster's network.
    pub async fn send_vote(
        &self,
        source: u64,
        target: u64,
        request: VoteRequest<C>,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        let mut network = Network {
            source,
            router: self.router.clone(),
        };
        let mut connection = network.new_client(target, &()).await;
        let option = RPCOption::new(Duration::from_secs(1));
        connection.vote(request, option).await
    }

    /// Fails every RPC between the two nodes of each link in `links`, and delivers all others.
    pub fn cut_links(&self, links: BTreeSet<(u64, u64)>) {
        let mut router = self.router.lock().unwrap();
        router.cut_links = links;
    }

    pub fn applied(&self, node_id: u64) -> Vec<EntryOf<C>> {
        self.state_machines[&node_id].applied()
    }

    /// Shuts node `node_id` down, then starts it again on its log store and state machine, which
    /// keep the log, the vote and the applied entries.
    pub async fn restart(&self, node_id: u64) -> Result<()> {
        let shutdown = self.raft(node_id).shutdown().await;
        shutdown.map_err(|error| anyhow!("node {node_id} fails to shut down: {error}"))?;

        let network = Network {
            source: node_id,
            router: self.router.clone(),
        };
        let log_store = self.log_stores[&node_id].clone();
        let state_machine = self.state_machines[&node_id].clone();
        let raft = Raft::new(node_id, self.config.clone(), network, log_store, state_machine).await?;

        let mut router = self.router.lock().unwrap();
        router.nodes.insert(node_id, raft);
        Ok(())
    }
}
