use std::panic;

use clap::Parser;
use openraft_jepsen_app::Opt;
use openraft_jepsen_app::start_raft_node;
use tracing_subscriber::EnvFilter;

const PANIC_MARKER: &str = "OPENRAFT_JEPSEN_PANIC";

fn install_panic_marker() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        eprintln!("{PANIC_MARKER}");
        default_hook(panic_info);
    }));
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    install_panic_marker();

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
