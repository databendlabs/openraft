use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Result;
use openraft::Config;
use openraft::Raft;
use openraft::RaftTypeConfig;
use openraft::alias::EntryOf;
use openraft::entry::RaftEntry;
use openraft::storage::RaftLogStorageExt;
use openraft::testing::log_id;

use crate::fixtures::custom_membership::log_store::LogStore;
use crate::fixtures::custom_membership::network::Network;
use crate::fixtures::custom_membership::network::Router;
use crate::fixtures::custom_membership::state_machine::StateMachine;

/// A type config that [`TestCluster`] can run.
pub trait TestConfig
where Self: RaftTypeConfig<NodeId = u64, Node = (), Term = u64, R = ()>
{
}

impl<C> TestConfig for C where C: RaftTypeConfig<NodeId = u64, Node = (), Term = u64, R = ()> {}

pub type TestRaft<C> = Raft<C, StateMachine<C>>;

/// Nodes of one membership, connected through one router.
pub struct TestCluster<C>
where C: TestConfig
{
    router: Arc<Mutex<Router<C>>>,
    state_machines: BTreeMap<u64, StateMachine<C>>,
}

impl<C> TestCluster<C>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    /// Starts the nodes `node_ids`. Each holds only `membership`, at log index 0, and no vote, so
    /// no node leads until the test triggers an election.
    pub async fn start(node_ids: BTreeSet<u64>, membership: C::Membership) -> Result<Self> {
        let config = Arc::new(
            Config {
                enable_tick: false,
                ..Default::default()
            }
            .validate()?,
        );
        let router = Arc::new(Mutex::new(Router {
            nodes: BTreeMap::new(),
            cut_links: BTreeSet::new(),
        }));
        let mut state_machines = BTreeMap::new();

        for node_id in node_ids {
            let mut log_store = LogStore::<C>::default();
            let initial_log_id = log_id::<C>(0, 0, 0);
            let membership_entry = EntryOf::<C>::new_membership(initial_log_id, membership.clone());
            log_store.blocking_append([membership_entry]).await?;

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

        Ok(Self { router, state_machines })
    }

    pub fn raft(&self, node_id: u64) -> TestRaft<C> {
        let router = self.router.lock().unwrap();
        router.nodes[&node_id].clone()
    }

    /// Fails every RPC between the two nodes of each link in `links`, and delivers all others.
    pub fn cut_links(&self, links: BTreeSet<(u64, u64)>) {
        let mut router = self.router.lock().unwrap();
        router.cut_links = links;
    }

    pub fn applied(&self, node_id: u64) -> Vec<EntryOf<C>> {
        self.state_machines[&node_id].applied()
    }
}
