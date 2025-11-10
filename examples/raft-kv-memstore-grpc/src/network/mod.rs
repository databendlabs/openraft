use openraft::error::NetworkError;
use openraft::error::Unreachable;
use openraft::network::v2::RaftNetworkV2;
use openraft::network::RPCOption;
use openraft::AnyError;
use openraft::LogId;
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

    /// Creates a gRPC channel to the target node.
    async fn create_channel(&self) -> Result<Channel, RPCError> {
        let server_addr = &self.target_node.rpc_addr;
        let channel = Channel::builder(format!("http://{}", server_addr).parse().unwrap())
            .connect()
            .await
            .map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))?;
        Ok(channel)
    }

    /// Checks if a gRPC error is due to payload being too large.
    fn is_payload_too_large(status: &tonic::Status) -> bool {
        status.code() == tonic::Code::OutOfRange
    }

    /// Sends append_entries in chunks when the payload is too large.
    async fn append_entries_chunked(&mut self, req: AppendEntriesRequest) -> Result<AppendEntriesResponse, RPCError> {
        const CHUNK_SIZE: usize = 2;

        let total_entries = req.entries.len();
        tracing::warn!(
            "Payload too large, splitting append_entries into chunks: target_node={:?}, total_entries={}, chunk_size={}",
            self.target_node,
            total_entries,
            CHUNK_SIZE
        );

        let mut current_offset = 0;
        let mut prev_log_id = req.prev_log_id;
        let mut last_response = None;

        while current_offset < total_entries {
            let channel = self.create_channel().await?;
            let mut client = RaftServiceClient::new(channel);

            let end_idx = (current_offset + CHUNK_SIZE).min(total_entries);
            let entries = req.entries[current_offset..end_idx].to_vec();

            tracing::warn!(
                "Sending append_entries chunk: chunk_start={}, chunk_end={}, chunk_entries={}",
                current_offset,
                end_idx,
                entries.len()
            );

            let chunk_req = AppendEntriesRequest {
                vote: req.vote,
                prev_log_id,
                leader_commit: req.leader_commit,
                entries: entries.clone(),
            };

            let response = client
                .append_entries(pb::AppendEntriesRequest::from(chunk_req))
                .await
                .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

            last_response = Some(response.into_inner());

            if let Some(last_entry) = entries.last() {
                prev_log_id = Some(LogId::new(last_entry.term, last_entry.index));
            }

            current_offset = end_idx;
        }

        tracing::warn!(
            "Completed chunked append_entries transmission: total_chunks={}",
            total_entries.div_ceil(CHUNK_SIZE)
        );

        Ok(AppendEntriesResponse::from(
            last_response.expect("At least one chunk should have been sent"),
        ))
    }

    /// Sends snapshot data in chunks through the provided channel.
    async fn send_snapshot_chunks(
        tx: &tokio::sync::mpsc::Sender<pb::SnapshotRequest>,
        snapshot_data: &[u8],
    ) -> Result<(), NetworkError> {
        let chunk_size = 1024 * 1024;
        for chunk in snapshot_data.chunks(chunk_size) {
            let request = pb::SnapshotRequest {
                payload: Some(pb::snapshot_request::Payload::Chunk(chunk.to_vec())),
            };
            tx.send(request).await.map_err(|e| NetworkError::new(&e))?;
        }
        Ok(())
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
        let channel = self.create_channel().await?;
        let mut client = RaftServiceClient::new(channel);

        let response = client.append_entries(pb::AppendEntriesRequest::from(req.clone())).await;

        match response {
            Ok(resp) => Ok(AppendEntriesResponse::from(resp.into_inner())),
            Err(status) if Self::is_payload_too_large(&status) => self.append_entries_chunked(req).await,
            Err(e) => Err(RPCError::Network(NetworkError::new(&e))),
        }
    }

    async fn full_snapshot(
        &mut self,
        vote: Vote,
        snapshot: Snapshot,
        _cancel: impl std::future::Future<Output = openraft::error::ReplicationClosed> + openraft::OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse, crate::typ::StreamingError> {
        let channel = self.create_channel().await?;
        let mut client = RaftServiceClient::new(channel);

        let (tx, rx) = tokio::sync::mpsc::channel(1024);
        let strm = ReceiverStream::new(rx);
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
        Self::send_snapshot_chunks(&tx, &snapshot.snapshot).await?;

        // 3. receive response

        let message = response.into_inner();

        Ok(SnapshotResponse {
            vote: message
                .vote
                .ok_or_else(|| NetworkError::new(&AnyError::error("Missing `vote` in snapshot response")))?,
        })
    }

    async fn vote(&mut self, req: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError> {
        let channel = self.create_channel().await?;
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
