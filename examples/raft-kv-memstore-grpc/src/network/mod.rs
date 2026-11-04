use std::future::Future;
use std::time::Duration;

use futures::SinkExt;
use futures::Stream;
use futures::StreamExt;
use futures::channel::mpsc;
use openraft::AnyError;
use openraft::OptionalSend;
use openraft::RaftNetworkFactory;
use openraft::base::BoxFuture;
use openraft::base::BoxStream;
use openraft::errors::NetworkError;
use openraft::errors::ReplicationClosed;
use openraft::errors::Unreachable;
use openraft::network::Backoff;
use openraft::network::NetBackoff;
use openraft::network::NetSnapshot;
use openraft::network::NetStreamAppend;
use openraft::network::NetTransferLeader;
use openraft::network::NetVote;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesResponseWithHint;
use openraft::raft::StreamAppendError;
use openraft::raft::StreamAppendResult;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use tonic::transport::Channel;

use crate::NodeId;
use crate::TypeConfig;
use crate::protobuf as pb;
use crate::protobuf::VoteRequest as PbVoteRequest;
use crate::protobuf::VoteResponse as PbVoteResponse;
use crate::protobuf::raft_service_client::RaftServiceClient;
use crate::typ::*;

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
    supports_hints: bool,
}

impl NetworkConnection {
    /// Creates a new NetworkConnection with the provided gRPC client.
    pub fn new(target_node: Node) -> Self {
        NetworkConnection {
            target_node,
            supports_hints: true,
        }
    }

    /// Creates a gRPC client to the target node.
    async fn make_client(&self) -> Result<RaftServiceClient<Channel>, RPCError> {
        let server_addr = &self.target_node.rpc_addr;
        let channel = Channel::builder(format!("http://{}", server_addr).parse().unwrap())
            .connect()
            .await
            .map_err(|e| RPCError::Unreachable(Unreachable::<TypeConfig>::new(&e)))?;
        Ok(RaftServiceClient::new(channel))
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

    /// Decode a reply from the optional `AppendEntriesWithHint` RPC.
    ///
    /// `Unimplemented` marks a peer without the RPC: return `None` and stop trying it on this
    /// connection. Every other failure is an error.
    fn handle_probe_response(
        &mut self,
        result: Result<tonic::Response<pb::AppendEntriesResponseWithHint>, tonic::Status>,
    ) -> Result<Option<AppendEntriesResponseWithHint<TypeConfig>>, RPCError> {
        let response = match result {
            Ok(response) => response.into_inner(),
            Err(error) if error.code() == tonic::Code::Unimplemented => {
                self.supports_hints = false;
                return Ok(None);
            }
            Err(error) => return Err(RPCError::Network(NetworkError::new(&error))),
        };

        let response = AppendEntriesResponseWithHint::try_from(response);
        let response = response.map_err(|error| RPCError::Network(NetworkError::new(&error)))?;

        Ok(Some(response))
    }

    /// Sends snapshot data in chunks through the provided channel.
    async fn send_snapshot_chunks(
        tx: &mut mpsc::Sender<pb::SnapshotRequest>,
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

// =============================================================================
// Sub-trait implementations for NetworkConnection
// =============================================================================
//
// Instead of implementing RaftNetworkV2 as a monolithic trait, this example
// demonstrates implementing individual sub-traits directly. This approach:
// - Shows exactly which network capabilities are provided
// - Each impl is focused on a single concern
// - gRPC's native bidirectional streaming maps naturally to NetStreamAppend

impl NetStreamAppend<TypeConfig> for NetworkConnection {
    fn append_entries_with_hint<'a>(
        &'a mut self,
        rpc: &'a AppendEntriesRequest,
        option: RPCOption,
    ) -> BoxFuture<'a, Result<Option<AppendEntriesResponseWithHint<TypeConfig>>, RPCError>> {
        Box::pin(async move {
            if !self.supports_hints {
                return Ok(None);
            }

            let mut client = self.make_client().await?;

            let payload = pb::AppendEntriesRequest::from(rpc.clone());
            let mut request = tonic::Request::new(payload);
            request.set_timeout(option.soft_ttl());

            let result = client.append_entries_with_hint(request).await;
            self.handle_probe_response(result)
        })
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
            let mut client = self.make_client().await?;

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
}

impl NetVote<TypeConfig> for NetworkConnection {
    async fn vote(&mut self, req: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError> {
        let mut client = self.make_client().await?;

        let proto_vote_req: PbVoteRequest = req.into();
        let response = client
            .vote(proto_vote_req)
            .await
            .map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&e)))?;

        let proto_vote_resp: PbVoteResponse = response.into_inner();
        Ok(proto_vote_resp.into())
    }

    async fn pre_vote(&mut self, req: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError> {
        let mut client = self.make_client().await?;

        let proto_vote_req: PbVoteRequest = req.into();
        let response = client
            .pre_vote(proto_vote_req)
            .await
            .map_err(|e| RPCError::Network(NetworkError::<TypeConfig>::new(&e)))?;

        let proto_vote_resp: PbVoteResponse = response.into_inner();
        Ok(proto_vote_resp.into())
    }
}

impl NetSnapshot<TypeConfig> for NetworkConnection {
    type SnapshotData = SnapshotData;

    async fn full_snapshot(
        &mut self,
        vote: Vote,
        snapshot: Snapshot,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse, StreamingError> {
        let mut client = self.make_client().await?;

        let (mut tx, rx) = mpsc::channel(1024);
        let response = client.snapshot(rx).await.map_err(|e| NetworkError::<TypeConfig>::new(&e))?;

        // 1. Send meta chunk
        let meta = &snapshot.meta;

        let request = pb::SnapshotRequest {
            payload: Some(pb::snapshot_request::Payload::Meta(pb::SnapshotRequestMeta {
                vote: Some(vote),
                last_log_id: meta.last_log_id.map(|log_id| log_id.into()),
                last_membership_log_id: meta.last_membership.log_id().map(|log_id| log_id.into()),
                last_membership: Some(meta.last_membership.membership().clone().into()),
            })),
        };

        tx.send(request).await.map_err(|e| NetworkError::<TypeConfig>::new(&e))?;

        // 2. Send data chunks
        Self::send_snapshot_chunks(&mut tx, &snapshot.snapshot).await?;

        // 3. Receive response
        let message = response.into_inner();

        Ok(SnapshotResponse {
            vote: message.vote.ok_or_else(|| {
                NetworkError::<TypeConfig>::new(&AnyError::error("Missing `vote` in snapshot response"))
            })?,
        })
    }
}

impl NetBackoff<TypeConfig> for NetworkConnection {
    fn backoff(&self) -> Option<Backoff> {
        Some(Backoff::new(std::iter::repeat(Duration::from_millis(200))))
    }
}

impl NetTransferLeader<TypeConfig> for NetworkConnection {
    async fn transfer_leader(
        &mut self,
        _req: TransferLeaderRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<TransferLeaderResponse<TypeConfig>, RPCError> {
        Err(RPCError::Unreachable(Unreachable::new(&AnyError::error(
            "transfer_leader not implemented",
        ))))
    }
}

#[cfg(test)]
mod tests {
    use openraft::raft::ConflictHint;

    use super::*;

    #[test]
    fn unimplemented_probe_disables_hints_for_legacy_peers() {
        let node = pb::Node {
            node_id: 1,
            rpc_addr: String::new(),
        };
        let mut network = NetworkConnection::new(node);
        let reply = Err(tonic::Status::unimplemented("legacy peer"));
        let actual = network.handle_probe_response(reply).unwrap();
        assert_eq!(None, actual);
        assert!(!network.supports_hints);
    }

    #[test]
    fn transport_failure_keeps_hint_support_and_surfaces_the_error() {
        let node = pb::Node {
            node_id: 1,
            rpc_addr: String::new(),
        };
        let mut network = NetworkConnection::new(node);
        let reply = Err(tonic::Status::unavailable("peer unavailable"));
        let actual = network.handle_probe_response(reply);
        let Err(RPCError::Network(_)) = actual else {
            panic!("expected the transport error, got {actual:?}");
        };
        assert!(network.supports_hints);
    }

    #[test]
    fn decoded_probe_reply_keeps_hint_support() {
        let node = pb::Node {
            node_id: 1,
            rpc_addr: String::new(),
        };
        let mut network = NetworkConnection::new(node);
        let hint = ConflictHint::new(vec![LogId::new(1, 2)]);
        let expected = AppendEntriesResponseWithHint::new(AppendEntriesResponse::Conflict, Some(hint));
        let reply = pb::AppendEntriesResponseWithHint::from(expected.clone());

        let actual = network.handle_probe_response(Ok(tonic::Response::new(reply))).unwrap();

        assert_eq!(Some(expected), actual);
        assert!(network.supports_hints);
    }

    #[test]
    fn undecodable_probe_reply_is_an_error_not_a_legacy_peer() {
        let node = pb::Node {
            node_id: 1,
            rpc_addr: String::new(),
        };
        let mut network = NetworkConnection::new(node);
        let reply = pb::AppendEntriesResponseWithHint {
            response: None,
            hint: None,
        };

        let actual = network.handle_probe_response(Ok(tonic::Response::new(reply)));

        let Err(RPCError::Network(_)) = actual else {
            panic!("expected the decoding error, got {actual:?}");
        };
        assert!(network.supports_hints);
    }

    #[test]
    fn peers_without_hint_support_are_not_contacted() {
        // An empty address makes any connection attempt fail.
        let node = pb::Node {
            node_id: 1,
            rpc_addr: String::new(),
        };
        let mut network = NetworkConnection::new(node);
        network.supports_hints = false;
        let request = AppendEntriesRequest {
            vote: pb::Vote {
                leader_id: Some(pb::LeaderId { term: 1, node_id: 1 }),
                committed: true,
            },
            prev_log_id: None,
            entries: vec![],
            leader_commit: None,
        };
        let option = RPCOption::new(Duration::from_secs(1));

        let probe = network.append_entries_with_hint(&request, option);
        let actual = futures::executor::block_on(probe);

        assert!(matches!(actual, Ok(None)));
    }
}
