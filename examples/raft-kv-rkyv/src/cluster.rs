use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use openraft::Config;
use openraft::async_runtime::WatchReceiver;
use openraft::impls::EmptyNode;

use crate::LogStore;
use crate::Raft;
use crate::network::Network;
use crate::state_machine::StateMachine;

pub(crate) struct Cluster {
    pub(crate) nodes: BTreeMap<u64, (Raft, StateMachine, LogStore)>,
    network: Network,
    config: Arc<Config>,
}

impl Cluster {
    pub(crate) async fn new() -> anyhow::Result<Self> {
        Self::start(None).await
    }

    pub(crate) async fn persistent(directory: &Path) -> anyhow::Result<Self> {
        anyhow::ensure!(
            directory.is_dir(),
            "data directory must already exist: {}",
            directory.display()
        );
        Self::start(Some(directory)).await
    }

    async fn start(directory: Option<&Path>) -> anyhow::Result<Self> {
        let network = Network::default();
        let config = Arc::new(
            Config {
                enable_tick: false,
                enable_heartbeat: false,
                enable_elect: false,
                enable_leader_restore: Some(false),
                // State and snapshots are volatile. Retain the entire log for restart replay.
                snapshot_policy: openraft::SnapshotPolicy::Never,
                ..Config::default()
            }
            .validate()?,
        );
        let mut nodes = BTreeMap::new();
        for id in 1..=3 {
            let log = match directory {
                None => LogStore::default(),
                Some(directory) => {
                    let path = directory.join(id.to_string());
                    std::fs::create_dir_all(&path)?;
                    std::fs::File::open(directory)?.sync_all()?;
                    LogStore::open(&path)?
                }
            };
            let state = StateMachine::default();
            let raft = Raft::new(id, config.clone(), network.clone(), log.clone(), state.clone()).await?;
            network.insert(id, raft.clone()).await;
            nodes.insert(id, (raft, state, log));
        }
        let cluster = Self { nodes, network, config };
        let leader = &cluster.nodes[&1].0;
        if leader.is_initialized().await? {
            let recovered_term = leader.metrics().borrow_watched().current_term;
            leader.trigger().elect(false).await?;
            // Startup metrics may still describe the recovered leadership. Require a new
            // election term as well as Leader state before issuing a client write.
            leader
                .wait(Some(Duration::from_secs(5)))
                .metrics(
                    |metrics| metrics.current_term > recovered_term && metrics.state == openraft::ServerState::Leader,
                    "won a new election after recovery",
                )
                .await?;
            leader
                .wait(Some(Duration::from_secs(5)))
                .leader_with_quorum_acked(None, "recovered leader can accept writes")
                .await?;
            return Ok(cluster);
        }
        leader.initialize(BTreeSet::from([1])).await?;
        leader.wait(Some(Duration::from_secs(5))).current_leader(1, "initial leader").await?;
        for id in [2, 3] {
            leader.add_learner(id, EmptyNode {}, true).await?;
        }
        leader.change_membership(BTreeSet::from([1, 2, 3]), false).await?;
        Ok(cluster)
    }

    pub(crate) async fn stop_node(&mut self, id: u64) -> anyhow::Result<()> {
        self.network.0.write().await.remove(&id);
        let (raft, _, _) = self.nodes.remove(&id).expect("running node");
        raft.shutdown().await?;
        Ok(())
    }

    pub(crate) async fn restart_node(&mut self, id: u64, directory: &Path) -> anyhow::Result<()> {
        let log = LogStore::open(&directory.join(id.to_string()))?;
        let state = StateMachine::default();
        let raft = Raft::new(
            id,
            self.config.clone(),
            self.network.clone(),
            log.clone(),
            state.clone(),
        )
        .await?;
        self.network.insert(id, raft.clone()).await;
        self.nodes.insert(id, (raft, state, log));
        Ok(())
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
