use std::sync::Arc;

use openraft::Config;
use tonic::transport::Server;
use tracing::info;

use crate::grpc::app_service::AppServiceImpl;
use crate::grpc::raft_service::RaftServiceImpl;
use crate::network::Network;
use crate::pb::app_service_server::AppServiceServer;
use crate::pb::raft_service_server::RaftServiceServer;
use crate::store::LogStore;
use crate::store::StateMachineStore;
use crate::typ::*;
use crate::NodeId;

pub async fn start_raft_app(node_id: NodeId, http_addr: String) -> Result<(), Box<dyn std::error::Error>> {
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

    // Start server with reduced max message size on Raft service to demonstrate chunking behavior.
    // The default gRPC message size limit is 4MB, but we set it to 1KB for the internal
    // Raft service to force the append_entries RPC to use chunking for even small payloads.
    // The app service uses the default limit since it's user-facing.
    let raft_service = RaftServiceServer::new(internal_service).max_decoding_message_size(1024);

    let server_future = Server::builder()
        .add_service(raft_service)
        .add_service(AppServiceServer::new(api_service))
        .serve(http_addr.parse()?);

    info!("Node {node_id} starting server at {http_addr}");
    server_future.await?;

    Ok(())
}
