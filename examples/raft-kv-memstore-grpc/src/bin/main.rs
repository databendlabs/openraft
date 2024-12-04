use std::sync::Arc;

use clap::Parser;
use openraft::Config;
use openraft_proto::internal_service::RaftInternalService;
use openraft_proto::management_service::RaftManagementService;
use openraft_proto::protobuf::internal_service_server::InternalServiceServer;
use openraft_proto::protobuf::management_service_server::ManagementServiceServer;
use raft_kv_memstore_grpc::network::Network;
use raft_kv_memstore_grpc::LogStore;
use raft_kv_memstore_grpc::Raft;
use raft_kv_memstore_grpc::StateMachineStore;
use raft_kv_memstore_grpc::StateMachineStoreWrapper;
use tonic::transport::Server;
use tracing::info;

#[derive(Parser, Clone, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Opt {
    #[clap(long)]
    pub id: u64,

    #[clap(long)]
    pub addr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse the parameters passed by arguments.
    let options = Opt::parse();
    let node_id = options.id;
    let addr = options.addr;

    // Initialize tracing with debug level
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    // Create a configuration for the raft instance.
    let config = Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        ..Default::default()
    };
    let config = Arc::new(config.validate().unwrap());

    // Create stores and network
    let log_store = LogStore::default();
    let state_machine_store = Arc::new(StateMachineStore::default());
    let state_machine_store = StateMachineStoreWrapper::new(state_machine_store);
    let network = Network {};

    // Create Raft instance
    let raft = Raft::new(node_id, config.clone(), network, log_store, state_machine_store).await?;
    let raft = Arc::new(raft);

    // Create the management service with raft instance
    let management_service = RaftManagementService::new(raft.clone()).await?;
    let internal_service = RaftInternalService::new(raft).await?;

    // Start server
    let server_future = Server::builder()
        .add_service(ManagementServiceServer::new(management_service))
        .add_service(InternalServiceServer::new(internal_service))
        .serve(addr.parse()?);

    info!("{} {} Server starting", node_id, addr);
    server_future.await?;

    Ok(())
}
