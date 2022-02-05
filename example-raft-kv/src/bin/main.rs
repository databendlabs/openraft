use std::sync::Arc;

use actix_web::middleware;
use actix_web::middleware::Logger;
use actix_web::web::Data;
use actix_web::App;
use actix_web::HttpServer;
use clap::Parser;
use env_logger::Env;
use example_raft_key_value::app::ExampleApp;
use example_raft_key_value::network::api;
use example_raft_key_value::network::management;
use example_raft_key_value::network::raft;
use example_raft_key_value::network::rpc::ExampleNetwork;
use example_raft_key_value::store::ExampleRequest;
use example_raft_key_value::store::ExampleResponse;
use example_raft_key_value::store::ExampleStore;
use openraft::Config;
use openraft::Raft;

pub type ExampleRaft = Raft<ExampleRequest, ExampleResponse, ExampleNetwork, ExampleStore>;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
    #[clap(long)]
    pub id: u64,

    #[clap(long)]
    pub http_addr: String,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Setup the logger
    env_logger::init_from_env(Env::default().default_filter_or("info"));

    // Parse the parameters passed by arguments.
    let options = Opt::parse();
    let node_id = options.id;

    // Create a configuration for the raft instance.
    let config = Arc::new(Config::default().validate().unwrap());

    // Create a instance of where the Raft data will be stored.
    let store = Arc::new(ExampleStore::default());

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = Arc::new(ExampleNetwork { store: store.clone() });

    // Create a local raft instance.
    let raft = Raft::new(node_id, config.clone(), network, store.clone());

    // Create an application that will store all the instances created above, this will
    // be later used on the actix-web services.
    let app = Data::new(ExampleApp {
        id: options.id,
        raft,
        store,
        config,
    });

    // Start the actix-web server.
    HttpServer::new(move || {
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
            .service(management::list_nodes)
            // application API
            .service(api::write)
            .service(api::read)
    })
    .bind(options.http_addr)?
    .run()
    .await
}
