use std::sync::Arc;

use actix_web::middleware;
use actix_web::middleware::Logger;
use actix_web::web::Data;
use actix_web::App;
use actix_web::HttpServer;
use clap::Parser;
use env_logger::Env;
use example_raft_kv::app::ExampleApp;
use example_raft_kv::network::ExampleNetwork;
use example_raft_kv::rpc_handlers;
use example_raft_kv::store::ExampleRequest;
use example_raft_kv::store::ExampleResponse;
use example_raft_kv::store::ExampleStore;
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
    env_logger::init_from_env(Env::default().default_filter_or("info"));

    let opt = Opt::parse();
    let node_id = opt.id;

    let config = Arc::new(Config::default().validate().unwrap());
    let store = Arc::new(ExampleStore::default());
    let network = Arc::new(ExampleNetwork { store: store.clone() });

    let raft = Raft::new(node_id, config.clone(), network, store.clone());

    let app = Data::new(ExampleApp {
        id: opt.id,
        raft,
        store,
        config,
    });

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .wrap(Logger::new("%a %{User-Agent}i"))
            .wrap(middleware::Compress::default())
            .app_data(app.clone())
            // raft internal RPC
            .service(rpc_handlers::append)
            .service(rpc_handlers::snapshot)
            .service(rpc_handlers::vote)
            // admin API
            .service(rpc_handlers::init)
            .service(rpc_handlers::add_learner)
            .service(rpc_handlers::change_membership)
            .service(rpc_handlers::metrics)
            .service(rpc_handlers::list_nodes)
            // application API
            .service(rpc_handlers::write)
            .service(rpc_handlers::read)
    })
    .bind(opt.http_addr)?
    .run()
    .await
}
