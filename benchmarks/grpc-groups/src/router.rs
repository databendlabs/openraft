use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Instant;

use openraft::OptionalSend;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::errors::Unreachable;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendError;
use openraft::raft::AppendResult;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::type_config::alias::SnapshotOf;
use openraft::type_config::alias::VoteOf;
use openraft_multi::GroupRouter;
use raft_kv_memstore_grpc::protobuf as raftpb;
use tonic::Code;
use tonic::Status;
use tonic::transport::Channel;
use tonic::transport::Endpoint;

use crate::C;
use crate::partition::Partition;
use crate::protobuf as pb;
use crate::protobuf::group_raft_client::GroupRaftClient;
use crate::router_stats::RouterStats;

/// Sends the Raft requests of every group on this node to the node of their target member, over
/// one gRPC channel per peer node, which all groups share, and counts them.
#[derive(Clone, Default)]
pub(crate) struct Router {
    inner: Arc<RouterInner>,
}

#[derive(Default)]
struct RouterInner {
    /// The client of each peer node, by node id, set once at setup.
    clients: OnceLock<BTreeMap<u64, GroupRaftClient<Channel>>>,
    stats: RouterStats,

    /// Holds the requests while this node is cut off from its peers.
    partition: Partition,
}

impl Router {
    /// Open a channel to each of `peers`, which maps a node id to its address. A channel connects
    /// on its first request.
    pub(crate) fn connect(&self, peers: &BTreeMap<u64, String>) -> anyhow::Result<()> {
        let mut clients = BTreeMap::new();
        for (node_id, addr) in peers {
            let channel = Endpoint::from_shared(format!("http://{addr}"))?.connect_lazy();
            clients.insert(*node_id, GroupRaftClient::new(channel));
        }

        if self.inner.clients.set(clients).is_err() {
            anyhow::bail!("the router connects only once");
        }
        Ok(())
    }

    pub(crate) fn stats(&self) -> &RouterStats {
        &self.inner.stats
    }

    /// The cut of this node, which the server holds the requests from the peers by too.
    pub(crate) fn partition(&self) -> &Partition {
        &self.inner.partition
    }

    fn client(&self, target: u64) -> Result<GroupRaftClient<Channel>, Unreachable<C>> {
        let client = self.inner.clients.get().and_then(|clients| clients.get(&target));
        client.cloned().ok_or_else(|| Unreachable::from_string(format!("no channel to node {target}")))
    }
}

impl GroupRouter<C, u64> for Router {
    type SnapshotData = raft_kv_memstore_grpc::SnapshotData;

    async fn append_entries(
        &self,
        target: u64,
        group_id: u64,
        rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        let stats = self.stats();
        stats.count_append();
        let heartbeat = rpc.entries.is_empty().then(|| HeartbeatTimer::start(stats));

        self.partition().hold().await;
        let mut client = self.client(target).map_err(RPCError::Unreachable)?;
        let request = pb::GroupAppendRequest {
            group_id,
            request: Some(rpc.into()),
        };
        stats.count_rpc();
        let response = client.append(request).await.map_err(rpc_error)?;

        if let Some(heartbeat) = heartbeat {
            heartbeat.replied();
        }
        append_result(response.into_inner())
    }

    async fn vote(
        &self,
        target: u64,
        group_id: u64,
        rpc: VoteRequest<C>,
        _option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        self.stats().count_vote();

        self.partition().hold().await;
        let mut client = self.client(target).map_err(RPCError::Unreachable)?;
        let request = pb::GroupVoteRequest {
            group_id,
            request: Some(rpc.into()),
        };
        let response = client.vote(request).await.map_err(rpc_error)?;
        Ok(response.into_inner().into())
    }

    async fn pre_vote(
        &self,
        target: u64,
        group_id: u64,
        rpc: VoteRequest<C>,
        _option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        self.stats().count_vote();

        self.partition().hold().await;
        let mut client = self.client(target).map_err(RPCError::Unreachable)?;
        let request = pb::GroupVoteRequest {
            group_id,
            request: Some(rpc.into()),
        };
        let response = client.pre_vote(request).await.map_err(rpc_error)?;
        Ok(response.into_inner().into())
    }

    async fn full_snapshot(
        &self,
        target: u64,
        group_id: u64,
        vote: VoteOf<C>,
        snapshot: SnapshotOf<C, Self::SnapshotData>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        self.partition().hold().await;
        let mut client = self.client(target).map_err(StreamingError::Unreachable)?;

        let membership = &snapshot.meta.last_membership;
        let meta = raftpb::SnapshotRequestMeta {
            vote: Some(vote),
            last_log_id: snapshot.meta.last_log_id.map(Into::into),
            last_membership_log_id: membership.log_id().map(Into::into),
            last_membership: Some(membership.membership().clone().into()),
        };
        let request = pb::GroupSnapshotRequest {
            group_id,
            meta: Some(meta),
            data: snapshot.snapshot,
        };
        let response = client.snapshot(request).await.map_err(streaming_error)?;

        let vote = response.into_inner().vote;
        let vote = vote.ok_or_else(|| StreamingError::Network(NetworkError::from_string("no vote in the reply")))?;
        Ok(SnapshotResponse { vote })
    }
}

/// Times one AppendEntries request without entries, a heartbeat or a probe, and counts it as
/// failed unless it gets a reply.
///
/// Openraft drops a heartbeat that has no reply within `heartbeat_interval`, which drops this timer
/// too.
struct HeartbeatTimer<'a> {
    stats: &'a RouterStats,
    started: Instant,
    replied: bool,
}

impl<'a> HeartbeatTimer<'a> {
    fn start(stats: &'a RouterStats) -> Self {
        Self {
            stats,
            started: Instant::now(),
            replied: false,
        }
    }

    fn replied(mut self) {
        self.replied = true;
        self.stats.heartbeat_replied(self.started.elapsed());
    }
}

impl Drop for HeartbeatTimer<'_> {
    fn drop(&mut self) {
        if !self.replied {
            self.stats.heartbeat_failed();
        }
    }
}

/// Convert the reply to an AppendEntries request, encoded as `examples/raft-kv-memstore-grpc`
/// encodes it.
fn append_result(response: raftpb::AppendEntriesResponse) -> Result<AppendResult<C>, RPCError<C>> {
    if let Some(higher_vote) = response.rejected_by {
        return Ok(Err(AppendError::HigherVote(higher_vote)));
    }

    if response.conflict {
        let log_id = response
            .last_log_id
            .ok_or_else(|| RPCError::Network(NetworkError::from_string("no conflicting log id in a conflict reply")))?;
        let conflict = AppendError::Conflict {
            log_id: log_id.into(),
            hint: response.hint.map(Into::into),
        };
        return Ok(Err(conflict));
    }

    Ok(Ok(response.last_log_id.map(Into::into)))
}

/// A node that is down refuses the connection, which tonic reports as `Unavailable`. Openraft
/// backs off from an unreachable target, rather than retrying it at once.
fn rpc_error(status: Status) -> RPCError<C> {
    if status.code() == Code::Unavailable {
        RPCError::Unreachable(Unreachable::new(&status))
    } else {
        RPCError::Network(NetworkError::new(&status))
    }
}

fn streaming_error(status: Status) -> StreamingError<C> {
    if status.code() == Code::Unavailable {
        StreamingError::Unreachable(Unreachable::new(&status))
    } else {
        StreamingError::Network(NetworkError::new(&status))
    }
}
