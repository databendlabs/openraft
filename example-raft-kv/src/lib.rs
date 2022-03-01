use std::sync::Arc;

use actix_web::middleware;
use actix_web::middleware::Logger;
use actix_web::web::Data;
use actix_web::App;
use actix_web::HttpServer;
use openraft::Config;
use openraft::Raft;

use crate::app::ExampleApp;
use crate::network::api;
use crate::network::management;
use crate::network::raft;
use crate::network::raft_network_impl::ExampleNetwork;
use crate::store::ExampleRequest;
use crate::store::ExampleResponse;
use crate::store::ExampleStore;

pub mod app;
pub mod client;
pub mod network;
pub mod store;

pub type ExampleNodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub ExampleTypeConfig: D = ExampleRequest, R = ExampleResponse, NodeId = ExampleNodeId
);

pub type ExampleRaft = Raft<ExampleTypeConfig, ExampleNetwork, Arc<ExampleStore>>;

pub async fn start_example_raft_node(node_id: ExampleNodeId, http_addr: String) -> std::io::Result<()> {
    // Create a configuration for the raft instance.
    let config = Arc::new(Config::default().validate().unwrap());

    // Create a instance of where the Raft data will be stored.
    let store = Arc::new(ExampleStore::default());

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = ExampleNetwork {};

    // Create a local raft instance.
    let raft = Raft::new(node_id, config.clone(), network, store.clone());

    // Create an application that will store all the instances created above, this will
    // be later used on the actix-web services.
    let app = Data::new(ExampleApp {
        id: node_id,
        addr: http_addr.clone(),
        raft,
        store,
        config,
    });

    // Start the actix-web server.
    let server = HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .wrap(Logger::new("%a %{User-Agent}i"))
            .wrap(middleware::Compress::default())
            .app_data(app.clone())
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
    });

    let x = server.bind(http_addr)?;

    x.run().await
}
