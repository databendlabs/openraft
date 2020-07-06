//! The Raft network interface.

use async_trait::async_trait;

use crate::{AppData, AppError};
use crate::raft::{AppendEntriesRequest, AppendEntriesResponse};
use crate::raft::{InstallSnapshotRequest, InstallSnapshotResponse};
use crate::raft::{VoteRequest, VoteResponse};

/// A trait defining the interface for a Raft network between cluster members.
///
/// See the [network chapter of the guide](TODO:)
/// for details and discussion on this trait and how to implement it.
#[async_trait]
pub trait RaftNetwork<D, E>: Send + Sync + 'static
    where
        D: AppData,
        E: AppError,
{
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn append_entries(&self, target: u64, msg: AppendEntriesRequest<D>) -> Result<AppendEntriesResponse, E>;

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    async fn install_snapshot(&self, target: u64, msg: InstallSnapshotRequest) -> Result<InstallSnapshotResponse, E>;

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn vote(&self, target: u64, msg: VoteRequest) -> Result<VoteResponse, E>;
}
