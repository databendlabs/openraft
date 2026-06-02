#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::sync::Arc;

use openraft::Config;
use openraft::NodeInfo as Node;

use crate::app::App;

pub mod app;
pub mod http_api;
pub mod store;
#[cfg(test)]
mod test;

pub type NodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = Node,
);

pub type LogStore = store::LogStore<TypeConfig>;
pub type StateMachineStore = store::StateMachineStore<TypeConfig>;
pub type Raft = openraft::Raft<TypeConfig, StateMachineStore>;

#[path = "../../utils/declare_types.rs"]
pub mod typ;

pub async fn start_example_raft_node(node_id: NodeId, api_addr: String, raft_addr: String) -> std::io::Result<()> {
    let (raft, state_machine_store) = new_raft_node(node_id).await;

    let app = Arc::new(App {
        id: node_id,
        api_addr: api_addr.clone(),
        raft_addr: raft_addr.clone(),
        raft,
        data: state_machine_store,
    });

    let raft_server = network_v2_http::Server::new(app.raft.clone()).run(raft_addr);
    let app_server = new_http_server(app).run(api_addr);

    tokio::try_join!(raft_server, app_server)?;
    Ok(())
}

/// Create the `Raft` instance from its building blocks. The `StateMachineStore` is returned
/// alongside so it can be reused as the application's data handle.
async fn new_raft_node(node_id: NodeId) -> (Raft, StateMachineStore) {
    // Create a configuration for the raft instance.
    let config = Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        ..Default::default()
    };
    let config = Arc::new(config.validate().unwrap());

    // Create an instance of where the Raft logs will be stored.
    let log_store = LogStore::default();
    // Create an instance of where the Raft data will be stored.
    let state_machine_store = StateMachineStore::default();

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = network_v2_http::NetworkFactory::new();

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store.clone()).await.unwrap();

    (raft, state_machine_store)
}

/// Build the application HTTP server: OpenRaft's admin/write routes plus this example's reads.
fn new_http_server(app: Arc<App>) -> app_http::Server<App> {
    app_http::Server::new(app)
        .add_openraft_routes()
        .post("/read", http_api::read)
        .post("/linearizable_read", http_api::linearizable_read)
        .post("/follower_read", http_api::follower_read)
}
