use clap::Parser;
use openraft::AsyncRuntime;
use raft_kv_memstore_grpc::TypeConfig;
use raft_kv_memstore_grpc::app::start_raft_app;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
    #[clap(long)]
    pub id: u64,

    #[clap(long)]
    /// Network address to bind the server to (e.g., "127.0.0.1:50051")
    pub addr: String,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize tracing first, before any logging happens
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_file(true)
        .with_line_number(true)
        .init();

    // Parse the parameters passed by arguments.
    let options = Opt::parse();

    let mut rt = <TypeConfig as openraft::RaftTypeConfig>::AsyncRuntime::new(1);
    rt.block_on(start_raft_app(options.id, options.addr))
}
