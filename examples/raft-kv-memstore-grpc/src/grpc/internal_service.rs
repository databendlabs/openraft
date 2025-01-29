use bincode::deserialize;
use bincode::serialize;
use futures::StreamExt;
use openraft::Snapshot;
use tonic::Request;
use tonic::Response;
use tonic::Status;
use tonic::Streaming;
use tracing::debug;

use crate::protobuf as pb;
use crate::protobuf::internal_service_server::InternalService;
use crate::protobuf::RaftReplyBytes;
use crate::protobuf::SnapshotRequest;
use crate::protobuf::VoteRequest;
use crate::protobuf::VoteResponse;
use crate::store::StateMachineData;
use crate::typ::*;

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
    raft_node: Raft,
}

impl InternalServiceImpl {
    /// Creates a new instance of the internal service
    ///
    /// # Arguments
    /// * `raft_node` - The Raft node instance this service will operate on
    pub fn new(raft_node: Raft) -> Self {
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
    async fn vote(&self, request: Request<VoteRequest>) -> Result<Response<VoteResponse>, Status> {
        debug!("Processing vote request");

        let vote_resp = self
            .raft_node
            .vote(request.into_inner().into())
            .await
            .map_err(|e| Status::internal(format!("Vote operation failed: {}", e)))?;

        debug!("Vote request processed successfully");
        Ok(Response::new(vote_resp.into()))
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
    async fn append_entries(
        &self,
        request: Request<pb::AppendEntriesRequest>,
    ) -> Result<Response<pb::AppendEntriesResponse>, Status> {
        debug!("Processing append entries request");

        let append_resp = self
            .raft_node
            .append_entries(request.into_inner().into())
            .await
            .map_err(|e| Status::internal(format!("Append entries operation failed: {}", e)))?;

        debug!("Append entries request processed successfully");
        Ok(Response::new(append_resp.into()))
    }

    /// Handles snapshot installation requests for state transfer using streaming.
    ///
    /// # Arguments
    /// * `request` - Stream of snapshot chunks with metadata
    ///
    /// # Returns
    /// * `Ok(Response)` - Response indicating success/failure of snapshot installation
    /// * `Err(Status)` - Error status if the snapshot operation fails
    async fn snapshot(&self, request: Request<Streaming<SnapshotRequest>>) -> Result<Response<RaftReplyBytes>, Status> {
        debug!("Processing streaming snapshot installation request");
        let mut stream = request.into_inner();

        // Get the first chunk which contains metadata
        let first_chunk = stream.next().await.ok_or_else(|| Status::invalid_argument("Empty snapshot stream"))??;

        // Deserialize the metadata from the first chunk
        let (vote, snapshot_meta) = Self::deserialize_request(&first_chunk.rpc_meta)?;

        // Prepare to collect snapshot data
        let mut snapshot_data_bytes = Vec::new();

        // Collect remaining chunks
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| Status::internal(format!("Failed to receive snapshot chunk: {}", e)))?;

            // Append non-empty chunks to snapshot data
            if !chunk.chunk.is_empty() {
                snapshot_data_bytes.extend_from_slice(&chunk.chunk);
            }
        }

        // Reconstruct StateMachineData from bytes
        let snapshot_data = match StateMachineData::from_bytes(&snapshot_data_bytes) {
            Ok(data) => data,
            Err(e) => return Err(Status::internal(format!("Failed to reconstruct snapshot data: {}", e))),
        };

        // Create snapshot from collected data
        let snapshot = Snapshot {
            meta: snapshot_meta,
            snapshot: Box::new(snapshot_data),
        };

        // Install the full snapshot
        let snapshot_resp = self
            .raft_node
            .install_full_snapshot(vote, snapshot)
            .await
            .map_err(|e| Status::internal(format!("Snapshot installation failed: {}", e)))?;

        debug!("Streaming snapshot installation request processed successfully");
        Self::create_response(snapshot_resp)
    }
}
