#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::path::Path;
use std::sync::Arc;

use openraft::Config;
use openraft::NodeInfo as Node;

use crate::app::App;
use crate::network::api;
use crate::store::new_storage;

pub mod app;
pub mod network;
pub mod store;

pub type NodeId = u64;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = Node,
);

pub type LogStore = openraft_rocksstore::log_store::RocksLogStore<TypeConfig>;
pub type StateMachineStore = store::StateMachineStore;
pub type Raft = openraft::Raft<TypeConfig, StateMachineStore>;

#[path = "../../utils/declare_types.rs"]
pub mod typ;

pub async fn start_example_raft_node<P>(
    node_id: NodeId,
    dir: P,
    api_addr: String,
    raft_addr: String,
) -> std::io::Result<()>
where
    P: AsRef<Path>,
{
    // Create a configuration for the raft instance.
    let config = Config {
        heartbeat_interval: 250,
        election_timeout_min: 299,
        ..Default::default()
    };

    let config = Arc::new(config.validate().unwrap());

    let (log_store, state_machine_store) = new_storage(&dir).await;

    let kvs = state_machine_store.data.kvs.clone();

    let network = network_v2_http::NetworkFactory::new();

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id, config.clone(), network, log_store, state_machine_store).await.unwrap();

    let app = Arc::new(App {
        id: node_id,
        api_addr: api_addr.clone(),
        raft_addr: raft_addr.clone(),
        raft,
        data: kvs,
    });

    let raft_server = network_v2_http::Server::new(app.raft.clone()).run(raft_addr);
    let app_server = app_http::Server::new(app)
        .post("/read", api::read)
        .post("/linearizable_read", api::linearizable_read)
        .run(api_addr);

    tokio::try_join!(raft_server, app_server)?;
    Ok(())
}
