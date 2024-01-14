use openraft::add_async_trait;
use openraft::error::InstallSnapshotError;
use openraft::error::RemoteError;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;

use crate::router::Router;
use crate::typ;
use crate::BasicNode;
use crate::NodeId;
use crate::TypeConfig;

pub struct Connection {
    router: Router,
    target: NodeId,
}

#[add_async_trait]
impl RaftNetworkFactory<TypeConfig> for Router {
    type Network = Connection;

    async fn new_client(&mut self, target: NodeId, _node: &BasicNode) -> Self::Network {
        Connection {
            router: self.clone(),
            target,
        }
    }
}

#[add_async_trait]
impl RaftNetwork<TypeConfig> for Connection {
    async fn send_append_entries(
        &mut self,
        req: AppendEntriesRequest<TypeConfig>,
    ) -> Result<AppendEntriesResponse<NodeId>, typ::RPCError> {
        let resp = self
            .router
            .send(self.target, "/raft/append", req)
            .await
            .map_err(|e| RemoteError::new(self.target, e))?;
        Ok(resp)
    }

    async fn send_install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<TypeConfig>,
    ) -> Result<InstallSnapshotResponse<NodeId>, typ::RPCError<InstallSnapshotError>> {
        let resp = self
            .router
            .send(self.target, "/raft/snapshot", req)
            .await
            .map_err(|e| RemoteError::new(self.target, e))?;
        Ok(resp)
    }

    async fn send_vote(&mut self, req: VoteRequest<NodeId>) -> Result<VoteResponse<NodeId>, typ::RPCError> {
        let resp = self
            .router
            .send(self.target, "/raft/vote", req)
            .await
            .map_err(|e| RemoteError::new(self.target, e))?;
        Ok(resp)
    }
}
