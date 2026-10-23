use std::collections::BTreeMap;
use std::sync::Arc;

use openraft::alias::LogIdListOf;
use openraft::async_runtime::WatchReceiver;
use openraft_rt::deterministic_rng::DeterministicRng;
use openraft_rt_tokio::TokioRuntime;
use turmoil::Sim;
use turmoil::net::TcpListener;

use crate::entry_history::EntryHistory;
use crate::network::TurmoilNetwork;
use crate::network::handle_rpc;
use crate::storage_model::StorageModel;
use crate::store::LogStore;
use crate::store::StateMachine;
use crate::store::StateMachineData;
use crate::typ::*;

/// Get the turmoil host name for a node.
pub fn host_name(id: NodeId) -> String {
    format!("node-{}", id)
}

/// Shared state for observing nodes from outside the simulation.
#[openraft::macros::since(version = "0.10.0", change = "has a private field: construct it with `new()`")]
pub struct ClusterState {
    /// Live Raft instances for each node (keyed by node ID).
    ///
    /// A crashed host's handle is removed from this map so external observers
    /// and simulated clients do not keep treating its last metrics snapshot as
    /// live state. When the host is bounced, `spawn_host()` recreates the Raft
    /// instance and re-inserts it.
    pub rafts: BTreeMap<NodeId, Arc<Raft>>,
    /// Log stores for each node.
    pub log_stores: BTreeMap<NodeId, Arc<LogStore>>,
    /// State machines for each node.
    pub state_machines: BTreeMap<NodeId, Arc<StateMachine>>,
    entry_history: Arc<std::sync::Mutex<EntryHistory>>,
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
            entry_history: Arc::new(std::sync::Mutex::new(EntryHistory::default())),
        }
    }

    /// Take violations of log entry identity observed since the previous check.
    #[openraft::macros::since(version = "0.10.0")]
    pub fn drain_log_violations(&self) -> Vec<String> {
        self.entry_history.lock().unwrap().drain_violations()
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

    pub fn get_all_durable_log_ids(&self) -> BTreeMap<NodeId, LogIdListOf<TypeConfig>> {
        self.log_stores.iter().map(|(&id, log_store)| (id, log_store.durable_log_id_list())).collect()
    }

    /// Find a Raft node that is currently the leader.
    pub fn find_leader(&self) -> Option<Arc<Raft>> {
        self.rafts.values().find(|raft| raft.metrics().borrow_watched().state.is_leader()).cloned()
    }

    /// Return the node id of the current leader, if any.
    pub fn find_leader_id(&self) -> Option<NodeId> {
        self.rafts
            .iter()
            .find(|(_, raft)| raft.metrics().borrow_watched().state.is_leader())
            .map(|(id, _)| *id)
    }

    /// Return the current leader together with its node id.
    pub fn find_leader_entry(&self) -> Option<(NodeId, Arc<Raft>)> {
        self.rafts
            .iter()
            .find(|(_, raft)| raft.metrics().borrow_watched().state.is_leader())
            .map(|(id, raft)| (*id, raft.clone()))
    }

    /// Get the live Raft handle of one node, if it is running.
    pub fn get_raft(&self, node_id: NodeId) -> Option<Arc<Raft>> {
        self.rafts.get(&node_id).cloned()
    }

    /// Get the state machine of one node (exists even while the node is down).
    pub fn get_state_machine(&self, node_id: NodeId) -> Option<Arc<StateMachine>> {
        self.state_machines.get(&node_id).cloned()
    }

    /// Register a freshly started Raft instance as live.
    pub fn register_raft(&mut self, node_id: NodeId, raft: Arc<Raft>) {
        self.rafts.insert(node_id, raft);
    }

    /// Drop the live handle for a crashed node.
    pub fn unregister_raft(&mut self, node_id: NodeId) {
        self.rafts.remove(&node_id);
    }
}

impl Default for ClusterState {
    fn default() -> Self {
        Self::new()
    }
}

/// Register a node's storage in the shared state BEFORE starting it.
///
/// `seed` drives the node's storage delays; give each node its own.
pub fn register_node_storage(
    node_id: NodeId,
    cluster_state: &Arc<std::sync::Mutex<ClusterState>>,
    model: &StorageModel,
    seed: u64,
) {
    let mut state = cluster_state.lock().unwrap();
    let history = state.entry_history.clone();
    if let std::collections::btree_map::Entry::Vacant(e) = state.log_stores.entry(node_id) {
        let log_store = LogStore::new(model.clone(), seed);
        let log_store = log_store.with_history(node_id, history);
        let state_machine = StateMachine::new(model, seed.wrapping_add(1));
        e.insert(Arc::new(log_store));
        state.state_machines.insert(node_id, Arc::new(state_machine));
    }
}

/// Create a Turmoil host for a node.
pub fn spawn_host(
    sim: &mut Sim,
    node_id: NodeId,
    raft_config: Arc<openraft::Config>,
    cluster_state: Arc<std::sync::Mutex<ClusterState>>,
    seed: u64,
    initial_nodes: BTreeMap<NodeId, Node>,
) {
    let host_name = host_name(node_id);
    sim.host(host_name, move || {
        let raft_config = raft_config.clone();
        let initial_nodes = initial_nodes.clone();
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

                    // Startup may await save_vote before publishing the recovered metrics.
                    raft.with_raft_state(|_| ()).await.expect("Failed to finish Raft startup");
                    let raft = Arc::new(raft);

                    cluster_state.lock().unwrap().register_raft(node_id, raft.clone());

                    // Node 1 initializes if needed
                    if node_id == 1 {
                        use openraft::storage::RaftLogStorage;
                        let mut log_store = cluster_state.lock().unwrap().log_stores.get(&node_id).unwrap().clone();
                        let is_initialized = log_store.get_log_state().await.unwrap().last_log_id.is_some();

                        if !is_initialized {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            tracing::info!("Initializing cluster on node {}", node_id);
                            raft.initialize(initial_nodes.clone()).await.expect("Failed to initialize");
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

/// Crash a node's software; it will stay down until `bounce_node` is called.
///
/// Use this together with a delayed `bounce_node` to model a downtime window
/// — `sim.bounce()` alone is an instant restart that only exercises the
/// software-restart / persistence-recovery path, not the "node unreachable
/// long enough to lose quorum" case.
///
/// A `power_loss` also drops every log write the node had not flushed.
pub fn crash_node(
    sim: &mut Sim,
    node_id: NodeId,
    cluster_state: &Arc<std::sync::Mutex<ClusterState>>,
    power_loss: bool,
) {
    let host_name = host_name(node_id);
    tracing::info!("CRASH: {} (power loss: {})", host_name, power_loss);
    sim.crash(host_name);
    let mut state = cluster_state.lock().unwrap();
    if let Some(log_store) = state.log_stores.get(&node_id) {
        log_store.crash(power_loss);
    }
    state.unregister_raft(node_id);
}

/// Restart a previously crashed node's software.
pub fn bounce_node(sim: &mut Sim, node_id: NodeId) {
    let host_name = host_name(node_id);
    tracing::info!("BOUNCE: {}", host_name);
    sim.bounce(host_name);
}
