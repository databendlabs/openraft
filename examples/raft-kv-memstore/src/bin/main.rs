use clap::Parser;
use raft_kv_memstore::start_example_raft_node;
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

    start_example_raft_node(options.id, options.api_addr, options.raft_addr).await
}
