use std::path::PathBuf;

use clap::Parser;
use raft_kv_log_wal_sm_mem::start_example_raft_node;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
struct Opt {
    #[clap(long)]
    id: u64,

    #[clap(long)]
    api_addr: String,

    #[clap(long)]
    raft_addr: String,

    #[clap(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(true)
        .with_level(true)
        .with_ansi(false)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let options = Opt::parse();
    let data_dir = options.data_dir.unwrap_or_else(|| PathBuf::from(format!("{}.wal", options.api_addr)));
    let data_dir = data_dir.display().to_string();

    start_example_raft_node(options.id, data_dir, options.api_addr, options.raft_addr).await
}
