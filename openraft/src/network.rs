//! The Raft network interface.

use std::fmt::Formatter;

use async_trait::async_trait;

use crate::error::InstallSnapshotError;
use crate::error::RPCError;
use crate::error::RaftError;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::RaftTypeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum RPCTypes {
    Vote,
    AppendEntries,
    InstallSnapshot,
}

impl std::fmt::Display for RPCTypes {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// A trait defining the interface for a Raft network between cluster members.
///
/// See the [network chapter of the guide](https://datafuselabs.github.io/openraft/getting-started.html#3-impl-raftnetwork)
/// for details and discussion on this trait and how to implement it.
///
/// Typically, the network implementation as such will be hidden behind a `Box<T>` or `Arc<T>` and
/// this interface implemented on the `Box<T>` or `Arc<T>`.
///
/// A single network instance is used to connect to a single target node. The network instance is
/// constructed by the [`RaftNetworkFactory`].
#[async_trait]
pub trait RaftNetwork<C>: Send + Sync + 'static
where C: RaftTypeConfig
{
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn send_append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
    ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>>;

    /// Send an InstallSnapshot RPC to the target Raft node (§7).
    async fn send_install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<C>,
    ) -> Result<
        InstallSnapshotResponse<C::NodeId>,
        RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
    >;

    /// Send a RequestVote RPC to the target Raft node (§5).
    async fn send_vote(
        &mut self,
        rpc: VoteRequest<C::NodeId>,
    ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>>;
}

/// A trait defining the interface for a Raft network factory to create connections between cluster
/// members.
///
/// See the [network chapter of the guide](https://datafuselabs.github.io/openraft/getting-started.html#3-impl-raftnetwork)
/// for details and discussion on this trait and how to implement it.
///
/// Typically, the network implementation as such will be hidden behind a `Box<T>` or `Arc<T>` and
/// this interface implemented on the `Box<T>` or `Arc<T>`.
#[async_trait]
pub trait RaftNetworkFactory<C>: Send + Sync + 'static
where C: RaftTypeConfig
{
    /// Actual type of the network handling a single connection.
    type Network: RaftNetwork<C>;

    /// Create a new network instance sending RPCs to the target node.
    ///
    /// This function should **not** create a connection but rather a client that will connect when
    /// required. Therefore there is chance it will build a client that is unable to send out
    /// anything, e.g., in case the Node network address is configured incorrectly. But this method
    /// does not return an error because openraft can only ignore it.
    ///
    /// The method is intentionally async to give the implementation a chance to use asynchronous
    /// sync primitives to serialize access to the common internal object, if needed.
    async fn new_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network;
}
