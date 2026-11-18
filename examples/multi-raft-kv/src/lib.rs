use std::sync::Arc;
use std::time::Duration;

use openraft::Config;
use openraft::type_config::TypeConfigExt;

use crate::app::Node;
use crate::network::NodeNetwork;
use crate::router::Router;

pub mod router;

pub mod api;
pub mod app;
pub mod append_batcher;
pub mod network;
pub mod store;

/// Node ID type - identifies a node in the cluster
pub type NodeId = u64;

/// Group ID type - identifies a Raft group
pub type GroupId = String;
pub type SnapshotData = std::io::Cursor<Vec<u8>>;

openraft::declare_raft_types!(
    /// Declare the type configuration for Multi-Raft K/V store.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
);

pub type LogStore = store::LogStore;
pub type StateMachineStore = sm_mem::StateMachineStore<TypeConfig>;
pub type Raft = openraft::Raft<TypeConfig, StateMachineStore>;

/// The wire format of AppendEntries replies, shared by the examples.
#[path = "../../utils/append_reply.rs"]
pub mod append_reply;

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

/// How often a leader sends heartbeats, in milliseconds.
const HEARTBEAT_INTERVAL: u64 = 500;

/// Create a Node with multiple Raft groups.
///
/// - One Node has ONE connection (shared by all groups)
/// - Each group has its own Raft instance
pub async fn create_node(node_id: NodeId, group_ids: &[GroupId], router: Router) -> Node {
    let (mut node, _tx) = Node::new(node_id, router.clone());
    let node_network = NodeNetwork::new(router.clone(), node.batcher.clone());

    for group_id in group_ids {
        let config = Config {
            heartbeat_interval: HEARTBEAT_INTERVAL,
            // The node's timer in `send_heartbeats()` triggers the heartbeats of all groups.
            enable_heartbeat: false,
            election_timeout_min: 1500,
            election_timeout_max: 3000,
            max_in_snapshot_log_to_keep: 0,
            enable_pre_vote: Some(true),
            ..Default::default()
        };

        let config = Arc::new(config.validate().unwrap());
        let log_store = LogStore::default();
        let state_machine_store = StateMachineStore::default();

        let network = network::NetworkFactory::new(node_network.clone(), group_id.clone());

        let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store.clone()).await.unwrap();

        node.add_group(group_id.clone(), raft, state_machine_store);
    }

    let rafts = node.groups.values().map(|group| group.raft.clone()).collect();
    TypeConfig::spawn(send_heartbeats(rafts));

    node
}

/// Trigger a heartbeat in each of `rafts` every [`HEARTBEAT_INTERVAL`], until all of them shut
/// down. Only a leader sends heartbeats: a follower ignores the trigger.
///
/// One timer for all groups makes their heartbeats to each node reach the [`AppendBatcher`]
/// together, so they share a batch. The timer of each group starts when that group becomes
/// leader, so heartbeats from separate timers share a batch only if the groups became leader
/// within one batching delay of each other.
///
/// [`AppendBatcher`]: crate::append_batcher::AppendBatcher
async fn send_heartbeats(rafts: Vec<Raft>) {
    loop {
        TypeConfig::sleep(Duration::from_millis(HEARTBEAT_INTERVAL)).await;

        let mut running = 0;
        for raft in &rafts {
            if raft.trigger().heartbeat().await.is_ok() {
                running += 1;
            }
        }
        if running == 0 {
            return;
        }
    }
}
