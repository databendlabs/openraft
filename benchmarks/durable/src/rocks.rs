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
    }
    if let Some(keep) = options.snapshot_keep {
        config.max_in_snapshot_log_to_keep = keep;
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
        .benchmark_routes("rocks", 0)
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn retention_override_does_not_require_a_snapshot_threshold() -> std::io::Result<()> {
        let options = Options::try_parse_from([
            "node",
            "--id",
            "1",
            "--api-addr",
            "127.0.0.1:1",
            "--raft-addr",
            "127.0.0.1:2",
            "--benchmark-control-addr",
            "127.0.0.1:3",
            "--data-dir",
            "unused",
            "--backend",
            "rocks",
            "--snapshot-keep",
            "7",
        ])
        .map_err(std::io::Error::other)?;
        let actual = config(&options)?;
        let default = rocks::example_config();
        assert_eq!(7, actual.max_in_snapshot_log_to_keep);
        assert_eq!(default.snapshot_policy, actual.snapshot_policy);
        Ok(())
    }
}
