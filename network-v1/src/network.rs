//! RaftNetwork trait for chunk-based snapshot transmission.
//!
//! This is the v1 network API that uses `install_snapshot()` for chunked snapshot transfer.
//! For new implementations, prefer `RaftNetworkV2` which uses `full_snapshot()`.

use std::time::Duration;

use openraft::OptionalSend;
use openraft::OptionalSync;
use openraft::RaftTypeConfig;
use openraft::error::InstallSnapshotError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::network::Backoff;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft_macros::add_async_trait;

/// A trait defining the interface for a Raft network between cluster members.
///
/// This is the v1 network API that uses chunk-based snapshot transmission via
/// `install_snapshot()`. For new implementations, consider using
/// [`RaftNetworkV2`](openraft::network::v2::RaftNetworkV2) instead.
///
/// To use this trait with openraft, wrap your implementation with
/// [`Adapter`](crate::Adapter) to convert it to `RaftNetworkV2`.
///
/// A single network instance is used to connect to a single target node. The network instance is
/// constructed by the [`RaftNetworkFactory`](`openraft::network::RaftNetworkFactory`).
#[add_async_trait]
pub trait RaftNetwork<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send an AppendEntries RPC to the target.
    ///
    /// This RPC is used for both log replication and heartbeats from the leader.
    /// Returns `RPCError::Unreachable` if the target node is unreachable.
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C, RaftError<C>>>;

    /// Send an InstallSnapshot RPC to the target.
    ///
    /// Transmits a snapshot chunk to help a lagging follower catch up.
    /// Called when a follower is too far behind to use log replication.
    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<C>,
        option: RPCOption,
    ) -> Result<InstallSnapshotResponse<C>, RPCError<C, RaftError<C, InstallSnapshotError>>>;

    /// Send a RequestVote RPC to the target.
    ///
    /// Used during leader election to request votes from other nodes.
    /// A candidate sends this to all cluster members to become leader.
    async fn vote(
        &mut self,
        rpc: VoteRequest<C>,
        option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C, RaftError<C>>>;

    /// Build a backoff instance if the target node is temporarily(or permanently) unreachable.
    ///
    /// When a [`Unreachable`](`openraft::error::Unreachable`) error is returned from the `Network`
    /// methods, Openraft does not retry connecting to a node immediately. Instead, it sleeps
    /// for a while and retries. The duration of the sleep is determined by the backoff
    /// instance.
    ///
    /// The backoff is an infinite iterator that returns the ith sleep interval before the ith
    /// retry. The returned instance will be dropped if a successful RPC is made.
    ///
    /// By default, it returns a constant backoff of 200 ms.
    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(200)))
    }
}
