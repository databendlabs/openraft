use std::collections::HashMap;
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
use openraft::raft::AppendError;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use openraft::type_config::TypeConfigExt;
use tonic::Code;
use tonic::Status;
use tonic::transport::Channel;
use tonic::transport::Endpoint;

use crate::NodeId;
use crate::TypeConfig;
use crate::protobuf as pb;
use crate::protobuf::VoteRequest as PbVoteRequest;
use crate::protobuf::VoteResponse as PbVoteResponse;
use crate::protobuf::raft_service_client::RaftServiceClient;
use crate::typ::*;

/// Builds the gRPC clients a node uses to reach the other nodes.
///
/// It keeps two channels for each peer address. A channel is one HTTP/2 connection, which tonic
/// opens on first use and opens again after a failure. Replication, votes and snapshots share the
/// first channel, and heartbeats use the second.
///
/// Heartbeats need a connection of their own because HTTP/2 flow control limits a whole connection
/// as well as each stream on it. A follower whose log flush is slow stops reading the replication
/// stream, and the data it has not read stays counted against the connection's window. Once that
/// window is used up, a heartbeat on the same connection waits until the follower reads again,
/// which may take as long as the flush. The follower itself answers a heartbeat at once: a
/// heartbeat waits for no log flush.
#[derive(Default)]
pub struct Network {
    /// The channels for replication, votes and snapshots, by peer address.
    channels: HashMap<String, Channel>,
    /// The channels for heartbeats, by peer address.
    heartbeat_channels: HashMap<String, Channel>,
}

impl Network {
    /// Return the channel to `addr` in `channels`, and create it on first use.
    ///
    /// A peer whose address changes, as by `ChangeMembers::SetNodes`, gets a new channel. A channel
    /// to an address no peer uses any more stays in the map.
    fn channel(channels: &mut HashMap<String, Channel>, addr: &str) -> Result<Channel, String> {
        if let Some(channel) = channels.get(addr) {
            return Ok(channel.clone());
        }

        let endpoint = Endpoint::from_shared(format!("http://{}", addr))
            .map_err(|e| format!("invalid address {}: {}", addr, e))?;
        let channel = endpoint.connect_lazy();
        channels.insert(addr.to_string(), channel.clone());
        Ok(channel)
    }
}

/// Implementation of the RaftNetworkFactory trait for creating new network connections.
/// This factory creates gRPC clients on the channels it keeps for each peer.
impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = NetworkConnection;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn new_client(&mut self, _: NodeId, node: &Node) -> Self::Network {
        let channel = Self::channel(&mut self.channels, &node.rpc_addr);
        NetworkConnection::new(node.clone(), channel)
    }

    /// Give heartbeats a channel of their own, so that they never wait behind replication.
    #[tracing::instrument(level = "debug", skip_all)]
    async fn new_heartbeat_client(&mut self, _: NodeId, node: &Node) -> Self::Network {
        let channel = Self::channel(&mut self.heartbeat_channels, &node.rpc_addr);
        NetworkConnection::new(node.clone(), channel)
    }
}

/// Represents an active network connection to a remote Raft node.
/// Handles serialization and deserialization of Raft messages over gRPC.
pub struct NetworkConnection {
    target_node: pb::Node,
    /// The channel to the target, or why the target's address gives none.
    channel: Result<Channel, String>,
}

impl NetworkConnection {
    /// Creates a NetworkConnection that sends its RPCs on `channel`.
    pub fn new(target_node: Node, channel: Result<Channel, String>) -> Self {
        NetworkConnection { target_node, channel }
    }

    /// Creates a gRPC client on the channel to the target node.
    fn client(&self) -> Result<RaftServiceClient<Channel>, RPCError> {
        match &self.channel {
            Ok(channel) => Ok(RaftServiceClient::new(channel.clone())),
            Err(e) => Err(RPCError::Unreachable(Unreachable::<TypeConfig>::from_string(e))),
        }
    }

    /// Wait for `call` until `option.soft_ttl()` ends.
    ///
    /// A channel connects on first use, so the deadline covers connecting as well.
    async fn with_deadline<T>(
        &self,
        option: &RPCOption,
        call: impl Future<Output = Result<T, Status>> + OptionalSend,
    ) -> Result<T, RPCError> {
        let ttl = option.soft_ttl();
        match TypeConfig::timeout(ttl, call).await {
            Ok(res) => res.map_err(status_to_rpc_error),
            Err(_elapsed) => Err(RPCError::Network(NetworkError::<TypeConfig>::from_string(format!(
                "no answer from node {} within {:?}",
                self.target_node.node_id, ttl
            )))),
        }
    }

    /// Convert pb::AppendEntriesResponse to AppendResult.
    ///
    /// A conflict is encoded as `conflict = true` plus a required `last_log_id` carrying the
    /// conflict log id, and an optional `hint`.
    fn pb_to_stream_result(resp: pb::AppendEntriesResponse) -> Result<AppendResult, RPCError> {
        if let Some(higher_vote) = resp.rejected_by {
            return Ok(Err(AppendError::HigherVote(higher_vote)));
        }

        if resp.conflict {
            let conflict_log_id = resp.last_log_id.ok_or_else(|| {
                RPCError::Network(NetworkError::<TypeConfig>::new(&AnyError::error(
                    "Missing `last_log_id` in conflict stream-append response",
                )))
            })?;
            let conflict = AppendError::Conflict {
                log_id: conflict_log_id.into(),
                hint: resp.hint.map(Into::into),
            };
            return Ok(Err(conflict));
        }

        Ok(Ok(resp.last_log_id.map(Into::into)))
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

/// Map a gRPC status to the error OpenRaft expects.
///
/// A connection that fails, or a peer that refuses it, is `Unavailable`, which is `Unreachable`
/// to OpenRaft, so that replication backs off. Any other status is a network error.
fn status_to_rpc_error(status: Status) -> RPCError {
    if status.code() == Code::Unavailable {
        RPCError::Unreachable(Unreachable::<TypeConfig>::new(&status))
    } else {
        RPCError::Network(NetworkError::<TypeConfig>::new(&status))
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
    fn stream_append<'s, S>(
        &'s mut self,
        input: S,
        option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<AppendResult, RPCError>>, RPCError>>
    where
        S: Stream<Item = AppendEntriesRequest> + OptionalSend + Unpin + 'static,
    {
        let fu = async move {
            let mut client = self.client()?;

            // The deadline covers opening the stream, not the answers: an answer waits for the
            // follower's log flush, which may take long while nothing is wrong.
            let opening = client.stream_append(input.map(pb::AppendEntriesRequest::from));
            let response = self.with_deadline(&option, opening).await?;

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
    async fn vote(&mut self, req: VoteRequest, option: RPCOption) -> Result<VoteResponse, RPCError> {
        let mut client = self.client()?;

        let proto_vote_req: PbVoteRequest = req.into();
        let response = self.with_deadline(&option, client.vote(proto_vote_req)).await?;

        let proto_vote_resp: PbVoteResponse = response.into_inner();
        Ok(proto_vote_resp.into())
    }

    async fn pre_vote(&mut self, req: VoteRequest, option: RPCOption) -> Result<VoteResponse, RPCError> {
        let mut client = self.client()?;

        let proto_vote_req: PbVoteRequest = req.into();
        let response = self.with_deadline(&option, client.pre_vote(proto_vote_req)).await?;

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
        let mut client = self.client()?;

        let (mut tx, rx) = mpsc::channel(1024);
        let response = client.snapshot(rx).await.map_err(status_to_rpc_error)?;

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
    use openraft::raft::AppendError;
    use openraft::raft::ConflictHint;
    use prost::Message;

    use super::NetworkConnection;
    use crate::protobuf as pb;
    use crate::typ::AppendResult;
    use crate::typ::LogId;

    /// A stream conflict keeps its hint across the wire.
    #[test]
    fn stream_conflict_hint_round_trip() -> Result<(), prost::DecodeError> {
        let committed = LogId::new(1, 2);
        let tail = LogId::new(2, 5);
        for log_ids in [vec![], vec![tail], vec![committed, tail]] {
            let conflict = AppendError::Conflict {
                log_id: LogId::new(2, 7),
                hint: Some(ConflictHint::new(log_ids)),
            };
            let expected: AppendResult = Err(conflict);

            let bytes = pb::AppendEntriesResponse::from(expected.clone()).encode_to_vec();
            let decoded = pb::AppendEntriesResponse::decode(bytes.as_slice())?;
            let actual = NetworkConnection::pb_to_stream_result(decoded).unwrap();

            assert_eq!(expected, actual);
        }
        Ok(())
    }

    /// A conflict from a peer that sends no hint decodes without one.
    #[test]
    fn stream_conflict_without_hint_decodes() {
        let response = pb::AppendEntriesResponse {
            rejected_by: None,
            conflict: true,
            last_log_id: Some(LogId::new(2, 7).into()),
            hint: None,
        };

        let actual = NetworkConnection::pb_to_stream_result(response).unwrap();

        let conflict = AppendError::Conflict {
            log_id: LogId::new(2, 7),
            hint: None,
        };
        let expected: AppendResult = Err(conflict);
        assert_eq!(expected, actual);
    }
}
