use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use openraft::Config;
use openraft::impls::EmptyNode;

use crate::LogStore;
use crate::Raft;
use crate::network::Network;
use crate::state_machine::StateMachine;

pub(crate) struct Cluster {
    pub(crate) nodes: BTreeMap<u64, (Raft, StateMachine, LogStore)>,
    network: Network,
}

impl Cluster {
    pub(crate) async fn new() -> anyhow::Result<Self> {
        let network = Network::default();
        let config = Arc::new(
            Config {
                enable_tick: false,
                enable_heartbeat: false,
                enable_elect: false,
                ..Config::default()
            }
            .validate()?,
        );
        let mut nodes = BTreeMap::new();
        for id in 1..=3 {
            let log = LogStore::default();
            let state = StateMachine::default();
            let raft = Raft::new(id, config.clone(), network.clone(), log.clone(), state.clone()).await?;
            network.insert(id, raft.clone()).await;
            nodes.insert(id, (raft, state, log));
        }
        let cluster = Self { nodes, network };
        let leader = &cluster.nodes[&1].0;
        leader.initialize(BTreeSet::from([1])).await?;
        leader.wait(Some(Duration::from_secs(5))).current_leader(1, "initial leader").await?;
        for id in [2, 3] {
            leader.add_learner(id, EmptyNode {}, true).await?;
        }
        leader.change_membership(BTreeSet::from([1, 2, 3]), false).await?;
        Ok(cluster)
    }

    pub(crate) async fn wait_applied(&self, index: u64) -> anyhow::Result<()> {
        for (id, (raft, _, _)) in &self.nodes {
            raft.wait(Some(Duration::from_secs(5)))
                .applied_index_at_least(Some(index), format!("node {id} applied archive"))
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn shutdown(self) -> anyhow::Result<()> {
        for (raft, _, _) in self.nodes.values() {
            raft.shutdown().await?;
        }
        self.network.clear().await;
        Ok(())
    }
}
