use std::sync::Arc;
use std::time::Duration;

use app_http::FOLLOWER_READ_TIMEOUT;
use app_http::FollowerReadError;
use raft_kv_log_wal_sm_mem as wal;
use wal::TypeConfig;
use wal::typ::Infallible;
use wal::typ::LinearizableReadError;

use crate::Options;
use crate::faults::Faults;
use crate::http_server::Server;
use crate::network::Factory;
use crate::slow_apply::SlowApply;

/// The example's in-memory state machine, with the delay of `--apply-delay-us` in every `apply()`.
type Machine = SlowApply<wal::StateMachineStore>;
type App = app_http::App<TypeConfig, Machine, wal::StateMachineStore>;

pub(crate) async fn run(options: &Options, faults: Arc<Faults>) -> std::io::Result<()> {
    let app = app(options, faults).await?;
    let rpc = network_v2_http::Server::new(app.raft.clone()).run(options.raft_addr.clone());
    let http = http(app, options.apply_delay_us).run(options.api_addr.clone());
    tokio::try_join!(http, rpc)?;
    Ok(())
}

async fn app(options: &Options, faults: Arc<Faults>) -> std::io::Result<Arc<App>> {
    let config = wal::example_config().validate().map_err(std::io::Error::other)?;
    let directory = options.data_dir.display();
    let log = wal::LogStore::open(directory)?;
    let data = wal::StateMachineStore::default();
    let machine = SlowApply::new(data.clone(), Duration::from_micros(options.apply_delay_us));
    let network = Factory::new(network_v2_http::NetworkFactory::new(), faults);
    let raft = openraft::Raft::new(options.id, Arc::new(config), network, log, machine)
        .await
        .map_err(std::io::Error::other)?;
    let app = App {
        id: options.id,
        api_addr: options.api_addr.clone(),
        raft_addr: options.raft_addr.clone(),
        raft,
        data,
    };
    Ok(Arc::new(app))
}

fn http(app: Arc<App>, apply_delay_us: u64) -> Server<App> {
    Server::new(app)
        .add_openraft_routes()
        .benchmark_routes("wal", apply_delay_us)
        .post("/read", read)
        .post("/linearizable_read", linearizable_read)
        .post("/follower_read", follower_read)
}

// The read handlers of `raft_kv_log_wal_sm_mem::http_api`, for an `App` whose state machine is
// wrapped in `SlowApply`.

async fn read(app: Arc<App>, key: String) -> Result<types_kv::Response, Infallible> {
    let value = app.data.get(&key).await;
    Ok(types_kv::Response { value })
}

async fn linearizable_read(app: Arc<App>, key: String) -> Result<types_kv::Response, LinearizableReadError> {
    app.ensure_linearizable().await?;
    let value = app.data.get(&key).await;
    Ok(types_kv::Response { value })
}

async fn follower_read(app: Arc<App>, key: String) -> Result<types_kv::Response, FollowerReadError<TypeConfig>> {
    app.ensure_follower_read_ready(FOLLOWER_READ_TIMEOUT).await?;
    let value = app.data.get(&key).await;
    Ok(types_kv::Response { value })
}
