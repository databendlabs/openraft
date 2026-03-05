use std::sync::Arc;

use openraft::Config;
use tokio::net::TcpListener;
use tracing::info;
use tracing::warn;

use crate::NodeId;
use crate::network::Network;
use crate::network::process_socket;
use crate::store::LogStore;
use crate::store::StateMachineStore;
use crate::typ::*;

pub async fn start_raft_app(
    node_id: NodeId,
    http_addr: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

    // Create Raft instance and keep it alive for the lifetime of this server task.
    let raft: Raft = Raft::new(node_id, config.clone(), network, log_store, state_machine_store.clone()).await?;

    // Bind tcp socket
    let listener = TcpListener::bind(&http_addr).await?;
    info!("Node {node_id} listening on {http_addr}");

    loop {
        let (socket, peer_addr) = listener.accept().await?;
        info!("Accepted incoming connection from {peer_addr}");

        let raft = raft.clone();
        tokio::spawn(async move {
            if let Err(e) = process_socket(socket, raft).await {
                warn!("Failed to process socket from {peer_addr}: {e}");
            }
        });
    }
}
