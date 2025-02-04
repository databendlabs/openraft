use std::sync::Arc;

use clap::Parser;
use openraft::Config;
use raft_kv_memstore_grpc::grpc::app_service::AppServiceImpl;
use raft_kv_memstore_grpc::grpc::raft_service::RaftServiceImpl;
use raft_kv_memstore_grpc::network::Network;
use raft_kv_memstore_grpc::protobuf::app_service_server::AppServiceServer;
use raft_kv_memstore_grpc::protobuf::raft_service_server::RaftServiceServer;
use raft_kv_memstore_grpc::typ::Raft;
use raft_kv_memstore_grpc::LogStore;
use raft_kv_memstore_grpc::StateMachineStore;
use tonic::transport::Server;
use tracing::info;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
    #[clap(long)]
    pub id: u64,

    #[clap(long)]
    /// Network address to bind the server to (e.g., "127.0.0.1:50051")
    pub addr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing first, before any logging happens
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_file(true)
        .with_line_number(true)
        .init();

    // Parse the parameters passed by arguments.
    let options = Opt::parse();
    let node_id = options.id;
    let addr = options.addr;

    // Create a configuration for the raft instance.
    let config = Arc::new(
        Config {
            heartbeat_interval: 500,
            election_timeout_min: 1500,
            election_timeout_max: 3000,
            ..Default::default()
        }
        .validate()?,
    );

    // Create stores and network
    let log_store = LogStore::default();
    let state_machine_store = Arc::new(StateMachineStore::default());
    let network = Network {};

    // Create Raft instance
    let raft = Raft::new(node_id, config.clone(), network, log_store, state_machine_store.clone()).await?;

    // Create the management service with raft instance
    let internal_service = RaftServiceImpl::new(raft.clone());
    let api_service = AppServiceImpl::new(raft, state_machine_store);

    // Start server
    let server_future = Server::builder()
        .add_service(RaftServiceServer::new(internal_service))
        .add_service(AppServiceServer::new(api_service))
        .serve(addr.parse()?);

    info!("Node {node_id} starting server at {addr}");
    server_future.await?;

    Ok(())
}
