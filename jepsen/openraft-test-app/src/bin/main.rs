use clap::Parser;
use openraft_jepsen_app::Opt;
use openraft_jepsen_app::start_raft_node;
use tracing_subscriber::EnvFilter;

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
    start_raft_node(options).await
}
