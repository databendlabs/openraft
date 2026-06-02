#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::io::Cursor;
use std::sync::Arc;

use openraft::Config;
use openraft::NodeInfo;

use crate::app::App;

pub mod api;
pub mod app;
pub mod store;

pub type NodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = NodeInfo,
        SnapshotData = Cursor<Vec<u8>>,
);

pub type LogStore = store::LogStore;
pub type StateMachineStore = sm_mem::StateMachineStore<TypeConfig>;
pub type Raft = openraft::Raft<TypeConfig, StateMachineStore>;

#[path = "../../utils/declare_types.rs"]
pub mod typ;

pub async fn new_raft_node(node_id: NodeId, api_addr: String, raft_addr: String) -> Arc<App> {
    // Create a configuration for the raft instance.
    let config = Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        // Once snapshot is built, delete the logs at once.
        // So that all further replication will be based on the snapshot.
        max_in_snapshot_log_to_keep: 0,
        ..Default::default()
    };

    let config = Arc::new(config.validate().unwrap());

    // Create a instance of where the Raft logs will be stored.
    let log_store = LogStore::default();

    // Create a instance of where the state machine data will be stored.
    let state_machine_store = StateMachineStore::default();

    // Create a local raft instance.
    let network = network_v2_http::NetworkFactory::new();

    let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store.clone()).await.unwrap();

    Arc::new(App {
        id: node_id,
        api_addr,
        raft_addr,
        raft,
        data: state_machine_store,
    })
}

pub async fn run_raft_node(app: Arc<App>) -> std::io::Result<()> {
    let api_addr = app.api_addr.clone();
    let raft_addr = app.raft_addr.clone();
    let raft = app.raft.clone();

    let raft_server = network_v2_http::Server::new(raft).run(raft_addr);
    let app_server = app_http::Server::new(app)
        .add_openraft_routes()
        .post("/read", api::read)
        .post("/linearizable_read", api::linearizable_read)
        .run(api_addr);

    tokio::try_join!(raft_server, app_server)?;
    Ok(())
}
