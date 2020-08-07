//! The Raft network interface.

use anyhow::Result;
use async_trait::async_trait;

use crate::AppData;
use crate::raft::{AppendEntriesRequest, AppendEntriesResponse};
use crate::raft::{InstallSnapshotRequest, InstallSnapshotResponse};
use crate::raft::{VoteRequest, VoteResponse};

/// A trait defining the interface for a Raft network between cluster members.
///
/// See the [network chapter of the guide](TODO:)
/// for details and discussion on this trait and how to implement it.
#[async_trait]
pub trait RaftNetwork<D>: Send + Sync + 'static
    where
        D: AppData,
{
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn append_entries(&self, target: u64, rpc: AppendEntriesRequest<D>) -> Result<AppendEntriesResponse>;

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    async fn install_snapshot(&self, target: u64, rpc: InstallSnapshotRequest) -> Result<InstallSnapshotResponse>;

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn vote(&self, target: u64, rpc: VoteRequest) -> Result<VoteResponse>;
}
