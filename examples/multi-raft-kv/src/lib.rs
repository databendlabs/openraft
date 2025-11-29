use std::sync::Arc;

use openraft::Config;

use crate::app::Node;
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

pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;

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

/// Create a Node with multiple Raft groups.
///
/// - One Node has ONE connection (shared by all groups)
/// - Each group has its own Raft instance
pub async fn create_node(node_id: NodeId, group_ids: &[GroupId], router: Router) -> Node {
    let (mut node, _tx) = Node::new(node_id, router.clone());

    for group_id in group_ids {
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

        let network = network::NetworkFactory::new(router.clone(), group_id.clone());

        let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store.clone()).await.unwrap();

        node.add_group(group_id.clone(), raft, state_machine_store);
    }

    node
}
