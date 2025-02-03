use openraft::error::NetworkError;
use openraft::error::Unreachable;
use openraft::network::v2::RaftNetworkV2;
use openraft::network::RPCOption;
use openraft::AnyError;
use openraft::RaftNetworkFactory;
use tonic::codegen::tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;

use crate::protobuf as pb;
use crate::protobuf::raft_service_client::RaftServiceClient;
use crate::protobuf::VoteRequest as PbVoteRequest;
use crate::protobuf::VoteResponse as PbVoteResponse;
use crate::typ::*;
use crate::NodeId;
use crate::TypeConfig;

/// Network implementation for gRPC-based Raft communication.
/// Provides the networking layer for Raft nodes to communicate with each other.
pub struct Network {}

impl Network {}

/// Implementation of the RaftNetworkFactory trait for creating new network connections.
/// This factory creates gRPC client connections to other Raft nodes.
impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = NetworkConnection;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn new_client(&mut self, _: NodeId, node: &Node) -> Self::Network {
        NetworkConnection::new(node.clone())
    }
}

/// Represents an active network connection to a remote Raft node.
/// Handles serialization and deserialization of Raft messages over gRPC.
pub struct NetworkConnection {
    target_node: pb::Node,
}

impl NetworkConnection {
    /// Creates a new NetworkConnection with the provided gRPC client.
    pub fn new(target_node: Node) -> Self {
        NetworkConnection { target_node }
    }
}

/// Implementation of RaftNetwork trait for handling Raft protocol communications.
#[allow(clippy::blocks_in_conditions)]
impl RaftNetworkV2<TypeConfig> for NetworkConnection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse, RPCError> {
        let server_addr = self.target_node.rpc_addr.clone();
        let channel = match Channel::builder(format!("http://{}", server_addr).parse().unwrap()).connect().await {
            Ok(channel) => channel,
            Err(e) => {
                return Err(RPCError::Unreachable(Unreachable::new(&e)));
            }
        };
        let mut client = RaftServiceClient::new(channel);

        let response = client
            .append_entries(pb::AppendEntriesRequest::from(req))
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let response = response.into_inner();
        Ok(AppendEntriesResponse::from(response))
    }

    async fn full_snapshot(
        &mut self,
        vote: Vote,
        snapshot: Snapshot,
        _cancel: impl std::future::Future<Output = openraft::error::ReplicationClosed> + openraft::OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse, crate::typ::StreamingError> {
        let server_addr = self.target_node.rpc_addr.clone();
        let channel = match Channel::builder(format!("http://{}", server_addr).parse().unwrap()).connect().await {
            Ok(channel) => channel,
            Err(e) => {
                return Err(RPCError::Unreachable(Unreachable::new(&e)).into());
            }
        };

        let (tx, rx) = tokio::sync::mpsc::channel(1024);
        let strm = ReceiverStream::new(rx);

        let mut client = RaftServiceClient::new(channel);
        let response = client.snapshot(strm).await.map_err(|e| NetworkError::new(&e))?;

        // 1. Send meta chunk

        let meta = &snapshot.meta;

        let request = pb::SnapshotRequest {
            payload: Some(pb::snapshot_request::Payload::Meta(pb::SnapshotRequestMeta {
                vote: Some(vote),
                last_log_id: meta.last_log_id.map(|log_id| log_id.into()),
                last_membership_log_id: meta.last_membership.log_id().map(|log_id| log_id.into()),
                last_membership: Some(meta.last_membership.membership().clone().into()),
                snapshot_id: meta.snapshot_id.to_string(),
            })),
        };

        tx.send(request).await.map_err(|e| NetworkError::new(&e))?;

        // 2. Send data chunks

        let chunk_size = 1024 * 1024;
        for chunk in snapshot.snapshot.as_ref().chunks(chunk_size) {
            let request = pb::SnapshotRequest {
                payload: Some(pb::snapshot_request::Payload::Chunk(chunk.to_vec())),
            };
            tx.send(request).await.map_err(|e| NetworkError::new(&e))?;
        }

        // 3. receive response

        let message = response.into_inner();

        Ok(SnapshotResponse {
            vote: message
                .vote
                .ok_or_else(|| NetworkError::new(&AnyError::error("Missing `vote` in snapshot response")))?,
        })
    }

    async fn vote(&mut self, req: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError> {
        let server_addr = self.target_node.rpc_addr.clone();
        let channel = match Channel::builder(format!("http://{}", server_addr).parse().unwrap()).connect().await {
            Ok(channel) => channel,
            Err(e) => {
                return Err(RPCError::Unreachable(Unreachable::new(&e)));
            }
        };
        let mut client = RaftServiceClient::new(channel);

        // Convert the openraft VoteRequest to protobuf VoteRequest
        let proto_vote_req: PbVoteRequest = req.into();

        // Create a tonic Request with the protobuf VoteRequest
        let request = tonic::Request::new(proto_vote_req);

        // Send the vote request
        let response = client.vote(request).await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        // Convert the response back to openraft VoteResponse
        let proto_vote_resp: PbVoteResponse = response.into_inner();
        Ok(proto_vote_resp.into())
    }
}
