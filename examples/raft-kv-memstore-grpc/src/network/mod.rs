use futures::Stream;
use futures::StreamExt;
use openraft::base::BoxFuture;
use openraft::base::BoxStream;
use openraft::error::NetworkError;
use openraft::error::Unreachable;
use openraft::network::v2::RaftNetworkV2;
use openraft::network::RPCOption;
use openraft::raft::StreamAppendError;
use openraft::raft::StreamAppendResult;
use openraft::AnyError;
use openraft::OptionalSend;
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
            .map_err(|e| RPCError::Unreachable(Unreachable::<TypeConfig>::new(&e)))?;
        Ok(channel)
    }

    /// Convert pb::AppendEntriesResponse to StreamAppendResult.
    ///
    /// For `StreamAppend`, conflict is encoded as `conflict = true` plus a required `last_log_id`
    /// carrying the conflict log id.
    fn pb_to_stream_result(resp: pb::AppendEntriesResponse) -> Result<StreamAppendResult<TypeConfig>, RPCError> {
        if let Some(higher_vote) = resp.rejected_by {
            return Ok(Err(StreamAppendError::HigherVote(higher_vote)));
        }

        if resp.conflict {
            let conflict_log_id = resp.last_log_id.ok_or_else(|| {
                RPCError::Network(NetworkError::<TypeConfig>::new(&AnyError::error(
                    "Missing `last_log_id` in conflict stream-append response",
                )))
            })?;
            return Ok(Err(StreamAppendError::Conflict(conflict_log_id.into())));
        }

        Ok(Ok(resp.last_log_id.map(Into::into)))
    }

    /// Sends snapshot data in chunks through the provided channel.
    async fn send_snapshot_chunks(
        tx: &tokio::sync::mpsc::Sender<pb::SnapshotRequest>,
        snapshot_data: &[u8],
    ) -> Result<(), NetworkError<TypeConfig>> {
        let chunk_size = 1024 * 1024;
        for chunk in snapshot_data.chunks(chunk_size) {
            let request = pb::SnapshotRequest {
                payload: Some(pb::snapshot_request::Payload::Chunk(chunk.to_vec())),
            };
            tx.send(request).await.map_err(|e| NetworkError::<TypeConfig>::new(&e))?;
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

        let response = client
            .append_entries(pb::AppendEntriesRequest::from(req))
            .await
            .map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&e)))?;

        Ok(AppendEntriesResponse::from(response.into_inner()))
    }

    fn stream_append<'s, S>(
        &'s mut self,
        input: S,
        _option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<StreamAppendResult<TypeConfig>, RPCError>>, RPCError>>
    where
        S: Stream<Item = AppendEntriesRequest> + OptionalSend + Unpin + 'static,
    {
        let fu = async move {
            let channel = self.create_channel().await?;
            let mut client = RaftServiceClient::new(channel);

            let response = client
                .stream_append(input.map(pb::AppendEntriesRequest::from))
                .await
                .map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&e)))?;

            let output = response.into_inner().map(|result| {
                let resp = result.map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&e)))?;
                Self::pb_to_stream_result(resp)
            });

            Ok(Box::pin(output) as BoxStream<'s, _>)
        };

        Box::pin(fu)
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
        let response = client.snapshot(strm).await.map_err(|e| NetworkError::<TypeConfig>::new(&e))?;

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

        tx.send(request).await.map_err(|e| NetworkError::<TypeConfig>::new(&e))?;

        // 2. Send data chunks
        Self::send_snapshot_chunks(&tx, &snapshot.snapshot).await?;

        // 3. receive response

        let message = response.into_inner();

        Ok(SnapshotResponse {
            vote: message.vote.ok_or_else(|| {
                NetworkError::<TypeConfig>::new(&AnyError::error("Missing `vote` in snapshot response"))
            })?,
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
        let response =
            client.vote(request).await.map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&e)))?;

        // Convert the response back to openraft VoteResponse
        let proto_vote_resp: PbVoteResponse = response.into_inner();
        Ok(proto_vote_resp.into())
    }
}
