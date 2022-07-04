use anyhow::Result;
use async_trait::async_trait;

use super::AppData;
use super::AppendEntriesRequest;
use super::AppendEntriesResponse;
use super::InstallSnapshotRequest;
use super::InstallSnapshotResponse;
use super::NodeId;
use super::VoteRequest;
use super::VoteResponse;

/// A trait defining the interface for a Raft network between cluster members.
///
/// See the [network chapter of the guide](https://datafuselabs.github.io/openraft/network.html)
/// for details and discussion on this trait and how to implement it.
#[async_trait]
pub trait RaftNetwork<D>: Send + Sync + 'static
where D: AppData
{
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn send_append_entries(&self, target: NodeId, rpc: AppendEntriesRequest<D>) -> Result<AppendEntriesResponse>;

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    async fn send_install_snapshot(
        &self,
        target: NodeId,
        rpc: InstallSnapshotRequest,
    ) -> Result<InstallSnapshotResponse>;

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn send_vote(&self, target: NodeId, rpc: VoteRequest) -> Result<VoteResponse>;
}
