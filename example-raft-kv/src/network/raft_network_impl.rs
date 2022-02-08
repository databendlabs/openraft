use std::sync::Arc;

use async_trait::async_trait;
use openraft::error::AppendEntriesError;
use openraft::error::InstallSnapshotError;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::RemoteError;
use openraft::error::VoteError;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::NodeId;
use openraft::RaftNetwork;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::store::ExampleRequest;
use crate::ExampleStore;

pub struct ExampleNetwork {
    pub store: Arc<ExampleStore>,
}

impl ExampleNetwork {
    pub async fn send_rpc<Req, Resp, Err>(&self, target: NodeId, uri: &str, req: Req) -> Result<Resp, RPCError<Err>>
    where
        Req: Serialize,
        Err: std::error::Error + DeserializeOwned,
        Resp: DeserializeOwned,
    {
        let addr = {
            let state_machine = self.store.state_machine.read().await;
            state_machine.nodes.get(&target).unwrap().clone()
        };

        let url = format!("http://{}/{}", addr, uri);
        let client = reqwest::Client::new();

        let resp = client.post(url).json(&req).send().await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        let res: Result<Resp, Err> = resp.json().await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        res.map_err(|e| RPCError::RemoteError(RemoteError::new(target, e)))
    }
}

#[async_trait]
impl RaftNetwork<ExampleRequest> for ExampleNetwork {
    async fn send_append_entries(
        &self,
        target: NodeId,
        req: AppendEntriesRequest<ExampleRequest>,
    ) -> Result<AppendEntriesResponse, RPCError<AppendEntriesError>> {
        self.send_rpc(target, "raft-append", req).await
    }

    async fn send_install_snapshot(
        &self,
        target: NodeId,
        req: InstallSnapshotRequest,
    ) -> Result<InstallSnapshotResponse, RPCError<InstallSnapshotError>> {
        self.send_rpc(target, "raft-snapshot", req).await
    }

    async fn send_vote(&self, target: NodeId, req: VoteRequest) -> Result<VoteResponse, RPCError<VoteError>> {
        self.send_rpc(target, "raft-vote", req).await
    }
}
