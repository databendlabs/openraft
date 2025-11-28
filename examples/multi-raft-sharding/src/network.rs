use std::future::Future;

use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::multi_raft::GroupNetworkAdapter;
use openraft::multi_raft::GroupNetworkFactory;
use openraft::multi_raft::GroupedRpc;
use openraft::network::Backoff;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::Snapshot;
use openraft::OptionalSend;

use crate::router::Router;
use crate::typ;
use crate::NodeId;
use crate::ShardId;
use crate::TypeConfig;

/// Shared connection that implements `GroupedRpc`.
#[derive(Clone)]
pub struct SharedConnection {
    router: Router,
    target: NodeId,
}

impl SharedConnection {
    pub fn new(router: Router, target: NodeId) -> Self {
        Self { router, target }
    }
}

impl GroupedRpc<TypeConfig, ShardId> for SharedConnection {
    async fn send_append_entries(
        &mut self,
        shard_id: ShardId,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<TypeConfig>, RPCError<TypeConfig>> {
        self.router.send(self.target, &shard_id, "/raft/append", rpc).await.map_err(RPCError::Unreachable)
    }

    async fn send_vote(
        &mut self,
        shard_id: ShardId,
        rpc: VoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        self.router.send(self.target, &shard_id, "/raft/vote", rpc).await.map_err(RPCError::Unreachable)
    }

    async fn send_snapshot(
        &mut self,
        shard_id: ShardId,
        vote: typ::Vote,
        snapshot: Snapshot<TypeConfig>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<TypeConfig>, StreamingError<TypeConfig>> {
        self.router
            .send(
                self.target,
                &shard_id,
                "/raft/snapshot",
                (vote, snapshot.meta, snapshot.snapshot),
            )
            .await
            .map_err(StreamingError::Unreachable)
    }

    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(std::time::Duration::from_millis(500)))
    }
}

/// Shard network factory that creates `GroupNetworkAdapter` instances.
pub type ShardNetworkFactory = GroupNetworkFactory<Router, ShardId>;

impl RaftNetworkFactory<TypeConfig> for ShardNetworkFactory {
    /// The network type is `GroupNetworkAdapter` wrapping `SharedConnection`.
    type Network = GroupNetworkAdapter<TypeConfig, ShardId, SharedConnection>;

    async fn new_client(&mut self, target: NodeId, _node: &openraft::BasicNode) -> Self::Network {
        let shared = SharedConnection::new(self.factory.clone(), target);
        GroupNetworkAdapter::new(shared, self.group_id.clone())
    }
}
