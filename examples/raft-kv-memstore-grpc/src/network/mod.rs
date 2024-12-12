use bincode::deserialize;
use bincode::serialize;
use openraft::error::NetworkError;
use openraft::network::v2::RaftNetworkV2;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::RaftNetworkFactory;
use tonic::transport::Channel;

use crate::protobuf::internal_service_client::InternalServiceClient;
use crate::protobuf::RaftRequestBytes;
use crate::protobuf::SnapshotRequest;
use crate::protobuf::VoteRequest as PbVoteRequest;
use crate::protobuf::VoteResponse as PbVoteResponse;
use crate::typ::RPCError;
use crate::Node;
use crate::NodeId;
use crate::TypeConfig;

/// Network implementation for gRPC-based Raft communication.
/// Provides the networking layer for Raft nodes to communicate with each other.
pub struct Network {}

type RaftServiceClient = InternalServiceClient<Channel>;

impl Network {}

/// Implementation of the RaftNetworkFactory trait for creating new network connections.
/// This factory creates gRPC client connections to other Raft nodes.
impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = NetworkConnection;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn new_client(&mut self, _: NodeId, node: &Node) -> Self::Network {
        let channel = Channel::builder(format!("http://{}", node.rpc_addr).parse().unwrap()).connect().await.unwrap();
        NetworkConnection::new(InternalServiceClient::new(channel))
    }
}

/// Represents an active network connection to a remote Raft node.
/// Handles serialization and deserialization of Raft messages over gRPC.
pub struct NetworkConnection {
    client: RaftServiceClient,
}

impl NetworkConnection {
    /// Creates a new NetworkConnection with the provided gRPC client.
    pub fn new(client: RaftServiceClient) -> Self {
        NetworkConnection { client }
    }
}

/// Implementation of RaftNetwork trait for handling Raft protocol communications.
#[allow(clippy::blocks_in_conditions)]
impl RaftNetworkV2<TypeConfig> for NetworkConnection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<TypeConfig>, RPCError> {
        let value = serialize(&req).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let request = RaftRequestBytes { value };
        let response =
            self.client.append_entries(request).await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let message = response.into_inner();
        let result = deserialize(&message.value).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        Ok(result)
    }

    async fn full_snapshot(
        &mut self,
        vote: openraft::Vote<<TypeConfig as openraft::RaftTypeConfig>::NodeId>,
        snapshot: openraft::Snapshot<TypeConfig>,
        _cancel: impl std::future::Future<Output = openraft::error::ReplicationClosed> + openraft::OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<openraft::raft::SnapshotResponse<TypeConfig>, crate::typ::StreamingError> {
        // Serialize the vote and snapshot metadata
        let rpc_meta =
            serialize(&(vote, snapshot.meta.clone())).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        // Convert snapshot data to bytes
        let snapshot_bytes = snapshot.snapshot.to_bytes();

        // Create a stream of snapshot requests
        let mut requests = Vec::new();

        // First request with metadata
        requests.push(SnapshotRequest {
            rpc_meta,
            chunk: Vec::new(), // First chunk contains only metadata
        });

        // Add snapshot data chunks
        let chunk_size = 1024 * 1024; // 1 MB chunks, adjust as needed
        for chunk in snapshot_bytes.chunks(chunk_size) {
            requests.push(SnapshotRequest {
                rpc_meta: Vec::new(), // Subsequent chunks have empty metadata
                chunk: chunk.to_vec(),
            });
        }

        // Create a stream from the requests
        let requests_stream = futures::stream::iter(requests);

        // Send the streaming snapshot request
        let response =
            self.client.snapshot(requests_stream).await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let message = response.into_inner();

        // Deserialize the response
        let result = deserialize(&message.value).map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        Ok(result)
    }

    async fn vote(
        &mut self,
        req: VoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError> {
        // Convert the openraft VoteRequest to protobuf VoteRequest
        let proto_vote_req: PbVoteRequest = req.into();

        // Create a tonic Request with the protobuf VoteRequest
        let request = tonic::Request::new(proto_vote_req);

        // Send the vote request
        let response = self.client.vote(request).await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        // Convert the response back to openraft VoteResponse
        let proto_vote_resp: PbVoteResponse = response.into_inner();
        Ok(proto_vote_resp.into())
    }
}
