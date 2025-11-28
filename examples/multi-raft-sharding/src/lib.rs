#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::sync::Arc;

use openraft::multi_raft::MultiRaftTypeConfig;
use openraft::Config;

use crate::app::App;
use crate::router::Router;
use crate::store::Request;
use crate::store::Response;
use crate::store::StateMachineData;

pub mod router;
pub mod shard_router;

pub mod api;
pub mod app;
pub mod network;
pub mod store;

/// Node ID type - identifies a physical node in the cluster.
pub type NodeId = u64;

/// Shard ID type - identifies a Raft group (shard).
///
/// In a sharded system, each shard is responsible for a range of keys.
/// The shard ID is used to route requests to the correct Raft group.
pub type ShardId = String;

openraft::declare_raft_types!(
    /// Type configuration for the sharded KV store.
    ///
    /// This configuration uses:
    /// - `Request`: Application requests including Set, Delete, and Split operations
    /// - `Response`: Application responses including split completion data
    /// - `StateMachineData`: The state machine snapshot format
    pub TypeConfig:
        D = Request,
        R = Response,
        SnapshotData = StateMachineData,
);

/// Extend TypeConfig with ShardId for Multi-Raft support.
impl MultiRaftTypeConfig for TypeConfig {
    type GroupId = ShardId;
}

pub type LogStore = store::LogStore;
pub type StateMachineStore = store::StateMachineStore;

/// Define all Raft-related type aliases.
#[path = "../../utils/declare_types.rs"]
pub mod typ;

/// Shard naming constants.
pub mod shards {
    /// The initial shard that contains all data.
    pub const SHARD_A: &str = "shard_a";

    /// The shard created after split (contains user_id > split_point).
    pub const SHARD_B: &str = "shard_b";
}

pub fn encode<T: serde::Serialize>(t: T) -> String {
    serde_json::to_string(&t).unwrap()
}

pub fn decode<T: serde::de::DeserializeOwned>(s: &str) -> T {
    serde_json::from_str(s).unwrap()
}

/// Create a new Raft instance for a specific shard on a node.
pub async fn new_raft(node_id: NodeId, shard_id: ShardId, router: Router) -> (typ::Raft, App) {
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

    let network = network::ShardNetworkFactory::new(shard_id.clone(), router.clone());

    let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store.clone()).await.unwrap();

    let app = App::new(node_id, shard_id, raft.clone(), router, state_machine_store);

    (raft, app)
}
