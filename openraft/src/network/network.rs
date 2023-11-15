use std::time::Duration;

use macros::add_async_trait;

use crate::error::InstallSnapshotError;
use crate::error::RPCError;
use crate::error::RaftError;
use crate::network::rpc_option::RPCOption;
use crate::network::Backoff;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;

/// A trait defining the interface for a Raft network between cluster members.
///
/// See the [network chapter of the guide](https://datafuselabs.github.io/openraft/getting-started.html#3-impl-raftnetwork)
/// for details and discussion on this trait and how to implement it.
///
/// A single network instance is used to connect to a single target node. The network instance is
/// constructed by the [`RaftNetworkFactory`](`crate::network::RaftNetworkFactory`).
///
/// ### 2023-05-03: New API with options
///
/// - This trait introduced 3 new API `append_entries`, `install_snapshot` and `vote` which accept
///   an additional argument [`RPCOption`], and deprecated the old API `send_append_entries`,
///   `send_install_snapshot` and `send_vote`.
///
/// - The old API will be **removed** in `0.9`. An application can still implement the old API
///   without any changes. Openraft calls only the new API and the default implementation will
///   delegate to the old API.
///
/// - Implementing the new APIs will disable the old APIs.
#[add_async_trait]
pub trait RaftNetwork<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send an AppendEntries RPC to the target.
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
        let _ = option;
        #[allow(deprecated)]
        self.send_append_entries(rpc).await
    }

    /// Send an InstallSnapshot RPC to the target.
    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<C>,
        option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<C::NodeId>,
        RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
    > {
        let _ = option;
        #[allow(deprecated)]
        self.send_install_snapshot(rpc).await
    }

    /// Send a RequestVote RPC to the target.
    async fn vote(
        &mut self,
        rpc: VoteRequest<C::NodeId>,
        option: RPCOption,
    ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
        let _ = option;
        #[allow(deprecated)]
        self.send_vote(rpc).await
    }

    /// Send an AppendEntries RPC to the target Raft node (§5).
    #[deprecated(note = "use `append_entries` instead. This method will be removed in 0.9")]
    async fn send_append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
    ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
        let _ = rpc;
        unimplemented!("send_append_entries is deprecated")
    }

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    #[deprecated(note = "use `install_snapshot` instead. This method will be removed in 0.9")]
    async fn send_install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<C>,
    ) -> Result<
        InstallSnapshotResponse<C::NodeId>,
        RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
    > {
        let _ = rpc;
        unimplemented!("send_install_snapshot is deprecated")
    }

    /// Send a RequestVote RPC to the target Raft node (§5).
    #[deprecated(note = "use `vote` instead. This method will be removed in 0.9")]
    async fn send_vote(
        &mut self,
        rpc: VoteRequest<C::NodeId>,
    ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
        let _ = rpc;
        unimplemented!("send_vote is deprecated")
    }

    /// Build a backoff instance if the target node is temporarily(or permanently) unreachable.
    ///
    /// When a [`Unreachable`](`crate::error::Unreachable`) error is returned from the `Network`
    /// methods, Openraft does not retry connecting to a node immediately. Instead, it sleeps
    /// for a while and retries. The duration of the sleep is determined by the backoff
    /// instance.
    ///
    /// The backoff is an infinite iterator that returns the ith sleep interval before the ith
    /// retry. The returned instance will be dropped if a successful RPC is made.
    ///
    /// By default it returns a constant backoff of 500 ms.
    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(500)))
    }
}
