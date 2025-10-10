#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]

use std::path::Path;
use std::sync::Arc;

use actix_web::middleware;
use actix_web::middleware::Logger;
use actix_web::web::Data;
use actix_web::HttpServer;
use openraft::Config;

use crate::app::App;
use crate::network::api;
use crate::network::management;
use crate::network::raft;
use crate::store::new_storage;
use crate::store::Request;
use crate::store::Response;

pub mod app;
pub mod network;
pub mod store;

pub type NodeId = u64;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
);

pub type LogStore = openraft_rocksstore::log_store::RocksLogStore<TypeConfig>;
pub type StateMachineStore = store::StateMachineStore;
pub type Raft = openraft::Raft<TypeConfig>;

#[path = "../../utils/declare_types.rs"]
pub mod typ;

pub async fn start_example_raft_node<P>(node_id: NodeId, dir: P, addr: String) -> std::io::Result<()>
where P: AsRef<Path> {
    // Create a configuration for the raft instance.
    let config = Config {
        heartbeat_interval: 250,
        election_timeout_min: 299,
        // RocksDB flush and send IO notification in another task, which does not guarantee the order.
        allow_io_notification_reorder: Some(true),
        ..Default::default()
    };

    let config = Arc::new(config.validate().unwrap());

    let (log_store, state_machine_store) = new_storage(&dir).await;

    let kvs = state_machine_store.data.kvs.clone();

    // Create the network layer using network-v1 crate
    let network = network_v1_http::NetworkFactory {};

    // Create a local raft instance.
    let raft = openraft::Raft::new(node_id, config.clone(), network, log_store, state_machine_store).await.unwrap();

    // Create an application that will store all the instances created above, this will
    // later be used on the actix-web services.
    let app_data = Data::new(App {
        id: node_id,
        addr: addr.clone(),
        raft,
        key_values: kvs,
        config,
    });

    // Start the actix-web server.
    let server = HttpServer::new(move || {
        actix_web::App::new()
            .wrap(Logger::default())
            .wrap(Logger::new("%a %{User-Agent}i"))
            .wrap(middleware::Compress::default())
            .app_data(app_data.clone())
            // raft internal RPC
            .service(raft::append)
            .service(raft::snapshot)
            .service(raft::vote)
            // admin API
            .service(management::init)
            .service(management::add_learner)
            .service(management::change_membership)
            .service(management::metrics)
            // application API
            .service(api::write)
            .service(api::read)
            .service(api::linearizable_read)
    });

    let x = server.bind(addr)?;

    x.run().await
}
