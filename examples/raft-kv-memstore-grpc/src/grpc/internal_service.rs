use bincode::deserialize;
use bincode::serialize;
use openraft::Raft;
use tonic::Request;
use tonic::Response;
use tonic::Status;
use tracing::debug;

use crate::protobuf::internal_service_server::InternalService;
use crate::protobuf::RaftReplyBytes;
use crate::protobuf::RaftRequestBytes;
use crate::TypeConfig;

/// Internal gRPC service implementation for Raft protocol communications.
/// This service handles the core Raft consensus protocol operations between cluster nodes.
///
/// # Responsibilities
/// - Vote requests/responses during leader election
/// - Log replication between nodes
/// - Snapshot installation for state synchronization
///
/// # Protocol Safety
/// This service implements critical consensus protocol operations and should only be
/// exposed to other trusted Raft cluster nodes, never to external clients.
pub struct InternalServiceImpl {
    /// The local Raft node instance that this service operates on
    raft_node: Raft<TypeConfig>,
}

impl InternalServiceImpl {
    /// Creates a new instance of the internal service
    ///
    /// # Arguments
    /// * `raft_node` - The Raft node instance this service will operate on
    pub fn new(raft_node: Raft<TypeConfig>) -> Self {
        InternalServiceImpl { raft_node }
    }

    /// Helper function to deserialize request bytes
    fn deserialize_request<T: for<'a> serde::Deserialize<'a>>(value: &[u8]) -> Result<T, Status> {
        deserialize(value).map_err(|e| Status::internal(format!("Failed to deserialize request: {}", e)))
    }

    /// Helper function to serialize response
    fn serialize_response<T: serde::Serialize>(value: T) -> Result<Vec<u8>, Status> {
        serialize(&value).map_err(|e| Status::internal(format!("Failed to serialize response: {}", e)))
    }

    /// Helper function to create a standard response
    fn create_response<T: serde::Serialize>(value: T) -> Result<Response<RaftReplyBytes>, Status> {
        let value = Self::serialize_response(value)?;
        Ok(Response::new(RaftReplyBytes { value }))
    }
}

#[tonic::async_trait]
impl InternalService for InternalServiceImpl {
    /// Handles vote requests during leader election.
    ///
    /// # Arguments
    /// * `request` - The vote request containing candidate information
    ///
    /// # Returns
    /// * `Ok(Response)` - Vote response indicating whether the vote was granted
    /// * `Err(Status)` - Error status if the vote operation fails
    ///
    /// # Protocol Details
    /// This implements the RequestVote RPC from the Raft protocol.
    /// Nodes vote for candidates based on log completeness and term numbers.
    async fn vote(&self, request: Request<RaftRequestBytes>) -> Result<Response<RaftReplyBytes>, Status> {
        debug!("Processing vote request");
        let req = request.into_inner();

        // Deserialize the vote request
        let vote_req = Self::deserialize_request(&req.value)?;

        // Process the vote request
        let vote_resp = self
            .raft_node
            .vote(vote_req)
            .await
            .map_err(|e| Status::internal(format!("Vote operation failed: {}", e)))?;

        debug!("Vote request processed successfully");
        Self::create_response(vote_resp)
    }

    /// Handles append entries requests for log replication.
    ///
    /// # Arguments
    /// * `request` - The append entries request containing log entries to replicate
    ///
    /// # Returns
    /// * `Ok(Response)` - Response indicating success/failure of the append operation
    /// * `Err(Status)` - Error status if the append operation fails
    ///
    /// # Protocol Details
    /// This implements the AppendEntries RPC from the Raft protocol.
    /// Used for both log replication and as heartbeat mechanism.
    async fn append(&self, request: Request<RaftRequestBytes>) -> Result<Response<RaftReplyBytes>, Status> {
        debug!("Processing append entries request");
        let req = request.into_inner();

        // Deserialize the append request
        let append_req = Self::deserialize_request(&req.value)?;

        // Process the append request
        let append_resp = self
            .raft_node
            .append_entries(append_req)
            .await
            .map_err(|e| Status::internal(format!("Append entries operation failed: {}", e)))?;

        debug!("Append entries request processed successfully");
        Self::create_response(append_resp)
    }

    /// Handles snapshot installation requests for state transfer.
    ///
    /// # Arguments
    /// * `request` - The snapshot installation request containing state data
    ///
    /// # Returns
    /// * `Ok(Response)` - Response indicating success/failure of snapshot installation
    /// * `Err(Status)` - Error status if the snapshot operation fails
    ///
    /// # Protocol Details
    /// This implements the InstallSnapshot RPC from the Raft protocol.
    /// Used to bring lagging followers up to date more efficiently than regular log replication.
    async fn snapshot(&self, request: Request<RaftRequestBytes>) -> Result<Response<RaftReplyBytes>, Status> {
        debug!("Processing snapshot installation request");
        let req = request.into_inner();

        // Deserialize the snapshot request
        let snapshot_req = Self::deserialize_request(&req.value)?;

        // Process the snapshot request
        let snapshot_resp = self
            .raft_node
            .install_snapshot(snapshot_req)
            .await
            .map_err(|e| Status::internal(format!("Snapshot installation failed: {}", e)))?;

        debug!("Snapshot installation request processed successfully");
        Self::create_response(snapshot_resp)
    }
}
