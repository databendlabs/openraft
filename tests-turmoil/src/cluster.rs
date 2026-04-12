use std::collections::BTreeMap;
use std::sync::Arc;

use openraft::async_runtime::WatchReceiver;
use openraft_rt::deterministic_rng::DeterministicRng;
use openraft_rt_tokio::TokioRuntime;
use turmoil::Sim;
use turmoil::net::TcpListener;

use crate::network::TurmoilNetwork;
use crate::network::handle_rpc;
use crate::store::LogStore;
use crate::store::StateMachine;
use crate::store::StateMachineData;
use crate::store::new_store;
use crate::typ::*;

/// Get the turmoil host name for a node.
pub fn host_name(id: NodeId) -> String {
    format!("node-{}", id)
}

/// Shared state for observing nodes from outside the simulation.
pub struct ClusterState {
    /// Raft instances for each node (keyed by node ID).
    pub rafts: BTreeMap<NodeId, Arc<Raft>>,
    /// Log stores for each node.
    pub log_stores: BTreeMap<NodeId, Arc<LogStore>>,
    /// State machines for each node.
    pub state_machines: BTreeMap<NodeId, Arc<StateMachine>>,
}

/// Combined snapshot of both Raft and State Machine state.
pub struct FullNodeSnapshot {
    pub raft: RaftMetrics,
    pub sm: StateMachineData,
}

impl ClusterState {
    pub fn new() -> Self {
        Self {
            rafts: BTreeMap::new(),
            log_stores: BTreeMap::new(),
            state_machines: BTreeMap::new(),
        }
    }

    /// Get metrics from all nodes.
    pub fn get_all_metrics(&self) -> Vec<(NodeId, RaftMetrics)> {
        self.rafts
            .iter()
            .map(|(&id, raft)| {
                let metrics = raft.metrics().borrow_watched().clone();
                (id, metrics)
            })
            .collect()
    }

    /// Get combined Raft and State Machine snapshots from all nodes.
    pub fn get_all_full_snapshots(&self) -> Vec<(NodeId, FullNodeSnapshot)> {
        self.rafts
            .iter()
            .map(|(&id, raft)| {
                let sm = self.state_machines.get(&id).expect("sm not found").get_data();
                let raft = raft.metrics().borrow_watched().clone();
                (id, FullNodeSnapshot { raft, sm })
            })
            .collect()
    }

    /// Find a Raft node that is currently the leader.
    pub fn find_leader(&self) -> Option<Arc<Raft>> {
        self.rafts
            .iter()
            .find(|(_, raft)| raft.metrics().borrow_watched().state.is_leader())
            .map(|(_, raft)| raft.clone())
    }
}

impl Default for ClusterState {
    fn default() -> Self {
        Self::new()
    }
}

/// Register a node's storage in the shared state BEFORE starting it.
pub fn register_node_storage(node_id: NodeId, cluster_state: &Arc<std::sync::Mutex<ClusterState>>) {
    let mut state = cluster_state.lock().unwrap();
    if let std::collections::btree_map::Entry::Vacant(e) = state.log_stores.entry(node_id) {
        let (log_store, state_machine) = new_store();
        e.insert(log_store);
        state.state_machines.insert(node_id, state_machine);
    }
}

/// Create a Turmoil host for a node.
pub fn spawn_host(
    sim: &mut Sim,
    node_id: NodeId,
    raft_config: Arc<openraft::Config>,
    cluster_state: Arc<std::sync::Mutex<ClusterState>>,
    seed: u64,
    all_nodes: BTreeMap<NodeId, Node>,
) {
    let host_name = host_name(node_id);
    sim.host(host_name, move || {
        let raft_config = raft_config.clone();
        let all_nodes = all_nodes.clone();
        let cluster_state = cluster_state.clone();
        let node_seed = seed.wrapping_add(node_id);

        async move {
            let res: Result<(), Box<dyn std::error::Error>> =
                DeterministicRng::<TokioRuntime>::scope(node_seed, async move {
                    let listener = TcpListener::bind("0.0.0.0:9000").await.expect("Failed to bind");
                    tracing::info!(node_id, "RPC server listening");

                    let (log_store, state_machine) = {
                        let state = cluster_state.lock().unwrap();
                        (
                            state.log_stores.get(&node_id).expect("node not registered").clone(),
                            state.state_machines.get(&node_id).expect("node not registered").clone(),
                        )
                    };

                    let raft = openraft::Raft::new(node_id, raft_config, TurmoilNetwork, log_store, state_machine)
                        .await
                        .expect("Failed to create Raft");

                    let raft = Arc::new(raft);

                    cluster_state.lock().unwrap().rafts.insert(node_id, raft.clone());

                    // Node 1 initializes if needed
                    if node_id == 1 {
                        use openraft::storage::RaftLogStorage;
                        let mut log_store = cluster_state.lock().unwrap().log_stores.get(&node_id).unwrap().clone();
                        let is_initialized = log_store.get_log_state().await.unwrap().last_log_id.is_some();

                        if !is_initialized {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            tracing::info!("Initializing cluster on node {}", node_id);
                            raft.initialize(all_nodes.clone()).await.expect("Failed to initialize");
                        }
                    }

                    loop {
                        match listener.accept().await {
                            Ok((stream, _addr)) => {
                                let raft_clone = raft.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = handle_rpc(raft_clone, stream).await {
                                        tracing::warn!("RPC handler error: {}", e);
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!("Accept error: {}", e);
                            }
                        }
                    }
                })
                .await;
            res
        }
    });
}

/// Restart a node by bouncing it.
pub fn restart_node(sim: &mut Sim, node_id: NodeId) {
    let host_name = host_name(node_id);
    tracing::info!("RESTART: bouncing {}", host_name);
    sim.bounce(host_name);
}
