#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::sync::Arc;

use openraft::multi_raft::MultiRaftTypeConfig;
use openraft::multi_raft::RaftRouter;
use openraft::Config;

use crate::app::App;
use crate::router::Router;
use crate::store::Request;
use crate::store::Response;
use crate::store::StateMachineData;

pub mod router;

pub mod api;
pub mod app;
pub mod network;
pub mod store;

/// Node ID type - identifies a node in the cluster
pub type NodeId = u64;

/// Group ID type - identifies a Raft group
pub type GroupId = String;

openraft::declare_raft_types!(
    /// Declare the type configuration for Multi-Raft K/V store.
    pub TypeConfig:
        D = Request,
        R = Response,
        // In this example, snapshot is just a copy of the state machine.
        SnapshotData = StateMachineData,
);

/// Extend TypeConfig with GroupId for Multi-Raft support.
/// Required for using `MultiRaftNetwork` and `GroupNetworkAdapter`.
impl MultiRaftTypeConfig for TypeConfig {
    type GroupId = GroupId;
}

pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;

/// Type alias for managing multiple Raft groups.
/// Uses openraft's `RaftRouter` for centralized Raft instance management.
pub type MultiRaftRouter = RaftRouter<TypeConfig>;

/// Define all Raft-related type aliases
#[path = "../../utils/declare_types.rs"]
pub mod typ;

pub mod groups {
    pub const USERS: &str = "users";
    pub const ORDERS: &str = "orders";
    pub const PRODUCTS: &str = "products";

    pub fn all() -> Vec<String> {
        vec![USERS.to_string(), ORDERS.to_string(), PRODUCTS.to_string()]
    }
}

pub fn encode<T: serde::Serialize>(t: T) -> String {
    serde_json::to_string(&t).unwrap()
}

pub fn decode<T: serde::de::DeserializeOwned>(s: &str) -> T {
    serde_json::from_str(s).unwrap()
}

/// Create a new Raft instance for a specific group on a node.
pub async fn new_raft(node_id: NodeId, group_id: GroupId, router: Router) -> (typ::Raft, App) {
    let config = Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        max_in_snapshot_log_to_keep: 0,
        ..Default::default()
    };

    let config = Arc::new(config.validate().unwrap());

    let log_store = LogStore::default();

    let state_machine_store = Arc::new(StateMachineStore::default());

    let network = network::GroupNetworkFactory::new(group_id.clone(), router.clone());

    let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store.clone()).await.unwrap();

    let app = App::new(node_id, group_id, raft.clone(), router, state_machine_store);

    (raft, app)
}
