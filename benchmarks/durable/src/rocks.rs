use std::sync::Arc;

use raft_kv_rocksdb as rocks;

use crate::Options;
use crate::faults::Faults;
use crate::http_server::Server;
use crate::network::Factory;

pub(crate) async fn run(options: &Options, faults: Arc<Faults>) -> std::io::Result<()> {
    let directory = options.data_dir.join("sm");
    let app = app(options, faults).await?;
    let rpc = rocks::network::Server::new(app.raft.clone(), directory).run(options.raft_addr.clone());
    let http = http(app).run(options.api_addr.clone());
    tokio::try_join!(http, rpc)?;
    Ok(())
}

fn config(options: &Options) -> std::io::Result<openraft::Config> {
    let mut config = rocks::example_config();
    if let Some(logs) = options.snapshot_logs {
        config.snapshot_policy = openraft::SnapshotPolicy::LogsSinceLast(logs);
        config.max_in_snapshot_log_to_keep = options.snapshot_keep;
    }
    config.validate().map_err(std::io::Error::other)
}

async fn app(options: &Options, faults: Arc<Faults>) -> std::io::Result<Arc<rocks::app::App>> {
    let config = config(options)?;
    let log = rocks::LogStore::open(options.data_dir.join("log"))?;
    let directory = options.data_dir.join("sm");
    let machine = rocks::StateMachineStore::open(&directory)?;
    let network = Factory::new(rocks::network::NetworkFactory::new(), faults);
    let raft = rocks::Raft::new(options.id, Arc::new(config), network, log, machine)
        .await
        .map_err(std::io::Error::other)?;
    let app = rocks::app::App {
        id: options.id,
        api_addr: options.api_addr.clone(),
        raft_addr: options.raft_addr.clone(),
        raft,
        data: (),
    };
    Ok(Arc::new(app))
}

fn http(app: Arc<rocks::app::App>) -> Server<rocks::app::App> {
    Server::new(app)
        .add_openraft_routes()
        .benchmark_routes("rocks")
        .post("/read", rocks::http_api::read)
        .post("/linearizable_read", rocks::http_api::linearizable_read)
        .post("/follower_read", follower_read)
}

async fn follower_read(
    app: Arc<rocks::app::App>,
    key: String,
) -> Result<types_kv::Response, app_http::FollowerReadError> {
    app.ensure_follower_read_ready().await?;
    let response = rocks::http_api::read(app, key).await.expect("infallible read handler");
    Ok(response)
}
