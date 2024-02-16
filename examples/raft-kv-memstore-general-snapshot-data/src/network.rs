use std::future::Future;



use openraft::error::RemoteError;
use openraft::error::ReplicationClosed;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;


use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::OptionalSend;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::Snapshot;
use openraft::Vote;

use crate::router::Router;
use crate::typ;
use crate::BasicNode;
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

    async fn snapshot(
        &mut self,
        vote: Vote<NodeId>,
        snapshot: Snapshot<TypeConfig>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<NodeId>, typ::StreamingError<typ::Fatal>> {
        let resp = self
            .router
            .send::<_, _, typ::Infallible>(self.target, "/raft/snapshot", (vote, snapshot.meta, snapshot.snapshot))
            .await
            .map_err(|e| RemoteError::new(self.target, e.into_fatal().unwrap()))?;
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
