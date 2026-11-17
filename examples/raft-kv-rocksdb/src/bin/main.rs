use std::path::PathBuf;

use clap::Parser;
use raft_kv_rocksdb::start_example_raft_node_with_config;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
#[openraft::macros::since(version = "0.10.1", change = "optional benchmark control and snapshot settings")]
pub struct Opt {
    #[clap(long)]
    pub id: u64,

    #[clap(long)]
    pub api_addr: String,

    #[clap(long)]
    pub raft_addr: String,

    #[clap(long, value_name = "PATH")]
    pub data_dir: Option<PathBuf>,
    /// Enable the separate benchmark partition-control listener.
    #[clap(long)]
    benchmark_control_addr: Option<String>,

    /// Build a checkpoint after this many logs (benchmark profile).
    #[clap(long)]
    snapshot_logs: Option<u64>,
    /// Retain this many snapshot-covered logs (benchmark profile).
    #[clap(long)]
    snapshot_keep: Option<u64>,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Setup the logger
    tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(true)
        .with_level(true)
        .with_ansi(false)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Parse the parameters passed by arguments.
    let options = Opt::parse();
    let data_dir = options.data_dir.unwrap_or_else(|| PathBuf::from(format!("{}.db", options.api_addr)));

    let mut config = raft_kv_rocksdb::example_config();
    if let Some(logs) = options.snapshot_logs {
        if logs == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "snapshot-logs must be positive",
            ));
        }
        config.snapshot_policy = openraft::SnapshotPolicy::LogsSinceLast(logs);
    }
    if let Some(keep) = options.snapshot_keep {
        config.max_in_snapshot_log_to_keep = keep;
    }
    if options.benchmark_control_addr.is_some() {
        app_http::benchmark_metrics::enable();
    }
    let faults = options.benchmark_control_addr.as_ref().map(|_| network_v2_http::Faults::install()).transpose()?;
    let node = start_example_raft_node_with_config(options.id, data_dir, options.api_addr, options.raft_addr, config);
    match (faults, options.benchmark_control_addr) {
        (Some(faults), Some(addr)) => {
            tokio::try_join!(node, faults.serve(addr))?;
            Ok(())
        }
        _ => node.await,
    }
}
