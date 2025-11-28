use std::future::Future;

use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::network::Backoff;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::Snapshot;
use openraft::OptionalSend;
use openraft_multi::GroupNetworkAdapter;
use openraft_multi::GroupNetworkFactory;
use openraft_multi::GroupRouter;

use crate::router::Router;
use crate::typ;
use crate::GroupId;
use crate::NodeId;
use crate::TypeConfig;

impl GroupRouter<TypeConfig, GroupId> for Router {
    async fn append_entries(
        &self,
        target: NodeId,
        group_id: GroupId,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<TypeConfig>, RPCError<TypeConfig>> {
        self.send(target, &group_id, "/raft/append", rpc).await.map_err(RPCError::Unreachable)
    }

    async fn vote(
        &self,
        target: NodeId,
        group_id: GroupId,
        rpc: VoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        self.send(target, &group_id, "/raft/vote", rpc).await.map_err(RPCError::Unreachable)
    }

    async fn full_snapshot(
        &self,
        target: NodeId,
        group_id: GroupId,
        vote: typ::Vote,
        snapshot: Snapshot<TypeConfig>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<TypeConfig>, StreamingError<TypeConfig>> {
        self.send(
            target,
            &group_id,
            "/raft/snapshot",
            (vote, snapshot.meta, snapshot.snapshot),
        )
        .await
        .map_err(StreamingError::Unreachable)
    }

    async fn transfer_leader(
        &self,
        target: NodeId,
        group_id: GroupId,
        req: TransferLeaderRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<(), RPCError<TypeConfig>> {
        self.send(target, &group_id, "/raft/transfer_leader", req).await.map_err(RPCError::Unreachable)
    }

    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(std::time::Duration::from_millis(500)))
    }
}

/// Network factory that creates `GroupNetworkAdapter` instances.
pub type NetworkFactory = GroupNetworkFactory<Router, GroupId>;

impl RaftNetworkFactory<TypeConfig> for NetworkFactory {
    /// The network type is `GroupNetworkAdapter` binding (Router, target, group_id).
    type Network = GroupNetworkAdapter<TypeConfig, GroupId, Router>;

    async fn new_client(&mut self, target: NodeId, _node: &openraft::BasicNode) -> Self::Network {
        GroupNetworkAdapter::new(self.factory.clone(), target, self.group_id.clone())
    }
}
