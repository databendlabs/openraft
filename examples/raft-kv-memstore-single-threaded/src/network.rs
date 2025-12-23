use openraft::error::InstallSnapshotError;
use openraft::network::RPCOption;
use openraft::BasicNode;
use openraft::RaftNetwork;
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

impl RaftNetwork<TypeConfig> for Connection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse, RPCError<RaftError>> {
        let resp = self.router.send(self.target, "/raft/append", req).await?;
        Ok(resp)
    }

    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse, RPCError<RaftError<InstallSnapshotError>>> {
        let resp = self.router.send(self.target, "/raft/snapshot", req).await?;
        Ok(resp)
    }

    async fn vote(&mut self, req: VoteRequest, _option: RPCOption) -> Result<VoteResponse, RPCError<RaftError>> {
        let resp = self.router.send(self.target, "/raft/vote", req).await?;
        Ok(resp)
    }
}
