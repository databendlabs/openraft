#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::sync::Arc;

use openraft::Config;
use openraft::NodeInfo as Node;

use crate::app::App;

pub mod app;
pub mod http_api;
pub mod store;

pub type NodeId = u64;

openraft::declare_raft_types!(
    /// The type config of the example K/V store, whose writes carry request IDs.
    pub TypeConfig:
        D = types_kv::DedupRequest,
        R = types_kv::Response,
        Node = Node,
);

pub type LogStore = store::LogStore<TypeConfig>;
pub type StateMachineStore = store::StateMachineStore<TypeConfig>;
pub type Raft = openraft::Raft<TypeConfig, StateMachineStore>;

pub async fn start_example_raft_node(node_id: NodeId, api_addr: String, raft_addr: String) -> std::io::Result<()> {
    let (raft, state_machine_store) = new_raft_node(node_id, example_config()).await;

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

/// The timing settings this example runs with.
pub fn example_config() -> Config {
    Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        enable_pre_vote: Some(true),
        ..Default::default()
    }
}

/// Create the `Raft` instance from its building blocks. The `StateMachineStore` is returned
/// alongside so it can be reused as the application's data handle.
async fn new_raft_node(node_id: NodeId, config: Config) -> (Raft, StateMachineStore) {
    let config = Arc::new(config.validate().unwrap());

    let log_store = LogStore::default();
    let state_machine_store = StateMachineStore::default();
    let network = network_v2_http::NetworkFactory::new();

    let raft = openraft::Raft::new(node_id, config, network, log_store, state_machine_store.clone()).await.unwrap();

    (raft, state_machine_store)
}

/// Build the application HTTP server: the OpenRaft routes of `app-http`, whose `/write` takes a
/// `types_kv::DedupRequest`, and this example's read.
fn new_http_server(app: Arc<App>) -> app_http::Server<App> {
    app_http::Server::new(app).add_openraft_routes().post("/read", http_api::read)
}

#[cfg(test)]
mod retry_test;
