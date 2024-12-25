use std::future::Future;

use openraft::error::ReplicationClosed;
use openraft::network::v2::RaftNetworkV2;
use openraft::network::RPCOption;
use openraft::BasicNode;
use openraft::OptionalSend;
use openraft::RaftNetworkFactory;

use crate::router::Router;
use crate::typ::*;
use crate::NodeId;
use crate::TypeConfig;

pub struct Connection {
    router: Router,
    target: NodeId,
}

impl RaftNetworkFactory<TypeConfig> for Router {
    type Network = Connection;

    async fn new_client(&mut self, target: NodeId, _node: &BasicNode) -> Self::Network {
        Connection {
            router: self.clone(),
            target,
        }
    }
}

impl RaftNetworkV2<TypeConfig> for Connection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse, RPCError> {
        let resp = self.router.send(self.target, "/raft/append", req).await?;
        Ok(resp)
    }

    /// A real application should replace this method with customized implementation.
    async fn full_snapshot(
        &mut self,
        vote: Vote,
        snapshot: Snapshot,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse, StreamingError> {
        let resp = self.router.send(self.target, "/raft/snapshot", (vote, snapshot.meta, snapshot.snapshot)).await?;
        Ok(resp)
    }

    async fn vote(&mut self, req: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError> {
        let resp = self.router.send(self.target, "/raft/vote", req).await?;
        Ok(resp)
    }
}
