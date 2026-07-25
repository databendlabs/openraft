use std::path::PathBuf;

use clap::Parser;
use raft_kv_rocksdb::start_example_raft_node;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
    #[clap(long)]
    pub id: u64,

    #[clap(long)]
    pub api_addr: String,

    #[clap(long)]
    pub raft_addr: String,

    #[clap(long, value_name = "PATH")]
    pub data_dir: Option<PathBuf>,
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

    start_example_raft_node(options.id, data_dir, options.api_addr, options.raft_addr).await
}
