use clap::Parser;
use env_logger::Env;
use example_raft_key_value::network::raft_network_impl::ExampleNetwork;
use example_raft_key_value::start_example_raft_node;
use example_raft_key_value::store::ExampleStore;
use example_raft_key_value::ExampleConfig;
use openraft::Raft;

pub type ExampleRaft = Raft<ExampleConfig, ExampleNetwork, ExampleStore>;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
    #[clap(long)]
    pub id: u64,

    #[clap(long)]
    pub http_addr: String,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Setup the logger
    env_logger::init_from_env(Env::default().default_filter_or("info"));

    // Parse the parameters passed by arguments.
    let options = Opt::parse();

    start_example_raft_node(options.id, options.http_addr).await
}
