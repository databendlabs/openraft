use std::sync::Arc;
use std::sync::Mutex;

use openraft::errors::RPCError;
use openraft::errors::RaftError;
use openraft::errors::RemoteError;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::vote::RaftLeaderIdExt;
use openraft_legacy::network_v1::ChunkedSnapshotReceiver;
use openraft_legacy::network_v1::InstallSnapshotError;
use openraft_legacy::network_v1::InstallSnapshotRequest;
use openraft_legacy::network_v1::InstallSnapshotResponse;
use openraft_legacy::network_v1::RaftNetwork;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemRaft;
use crate::fixtures::RaftRouter;

/// Records actual chunks delivered by the legacy snapshot adapter.
pub struct ChunkedNetwork {
    pub node: MemRaft,
    pub owner: RaftRouter,
    pub requests: Arc<Mutex<Vec<InstallSnapshotRequest<TypeConfig>>>>,
}

impl RaftNetwork<TypeConfig> for ChunkedNetwork {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<TypeConfig>, RPCError<TypeConfig, RaftError<TypeConfig>>> {
        let response = self.node.append_entries(rpc).await;
        response.map_err(|error| {
            let node_id = *self.node.node_id();
            let remote = RemoteError::new(node_id, error);
            RPCError::RemoteError(remote)
        })
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse<TypeConfig>, RPCError<TypeConfig, RaftError<TypeConfig, InstallSnapshotError>>>
    {
        let from = rpc.vote.leader_id().to_node_id();
        let to = *self.node.node_id();
        self.owner.before_rpc(rpc.clone(), from, to).await?;
        {
            let recorded = rpc.clone();
            let mut requests = self.requests.lock().unwrap();
            requests.push(recorded);
        }
        let response = self.node.install_snapshot(rpc.clone()).await;
        let response = response.map_err(|error| {
            let remote = RemoteError::new(to, error);
            RPCError::RemoteError(remote)
        })?;
        let hook_response = InstallSnapshotResponse { vote: response.vote };
        self.owner.call_rpc_post_hook(rpc, hook_response, from, to).await?;
        Ok(response)
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig, RaftError<TypeConfig>>> {
        let response = self.node.vote(rpc).await;
        response.map_err(|error| {
            let node_id = *self.node.node_id();
            let remote = RemoteError::new(node_id, error);
            RPCError::RemoteError(remote)
        })
    }
}
