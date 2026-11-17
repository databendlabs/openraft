use std::sync::Arc;

use raft_kv_log_wal_sm_mem as wal;

use crate::Options;
use crate::faults::Faults;
use crate::http_server::Server;
use crate::network::Factory;

pub(crate) async fn run(options: &Options, faults: Arc<Faults>) -> std::io::Result<()> {
    let app = app(options, faults).await?;
    let rpc = network_v2_http::Server::new(app.raft.clone()).run(options.raft_addr.clone());
    let http = http(app).run(options.api_addr.clone());
    tokio::try_join!(http, rpc)?;
    Ok(())
}

async fn app(options: &Options, faults: Arc<Faults>) -> std::io::Result<Arc<wal::app::App>> {
    let config = wal::example_config().validate().map_err(std::io::Error::other)?;
    let directory = options.data_dir.display();
    let log = wal::LogStore::open(directory)?;
    let machine = wal::StateMachineStore::default();
    let network = Factory::new(network_v2_http::NetworkFactory::new(), faults);
    let raft = wal::Raft::new(options.id, Arc::new(config), network, log, machine.clone())
        .await
        .map_err(std::io::Error::other)?;
    let app = wal::app::App {
        id: options.id,
        api_addr: options.api_addr.clone(),
        raft_addr: options.raft_addr.clone(),
        raft,
        data: machine,
    };
    Ok(Arc::new(app))
}

fn http(app: Arc<wal::app::App>) -> Server<wal::app::App> {
    Server::new(app)
        .add_openraft_routes()
        .benchmark_routes("wal")
        .post("/read", wal::http_api::read)
        .post("/linearizable_read", wal::http_api::linearizable_read)
        .post("/follower_read", wal::http_api::follower_read)
}
