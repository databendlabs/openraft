use std::future::Future;
use std::time::Duration;

use openraft_macros::add_async_trait;

use crate::error::Fatal;
use crate::error::RPCError;
use crate::error::RaftError;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::network::rpc_option::RPCOption;
use crate::network::Backoff;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::Vote;

/// A trait defining the interface for a Raft network between cluster members.
///
/// See the [network chapter of the guide](crate::docs::getting_started#4-implement-raftnetwork)
/// for details and discussion on this trait and how to implement it.
///
/// A single network instance is used to connect to a single target node. The network instance is
/// constructed by the [`RaftNetworkFactory`](`crate::network::RaftNetworkFactory`).
///
/// [Ensure connection to correct node][correct-node]
///
/// [`RaftNetwork`]: crate::network::v1::RaftNetwork
/// [correct-node]: `crate::docs::cluster_control::dynamic_membership#ensure-connection-to-the-correct-node`
#[add_async_trait]
pub trait RaftNetwork<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send an AppendEntries RPC to the target.
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>>;

    /// Send an InstallSnapshot RPC to the target.
    #[cfg(feature = "generic-snapshot-data")]
    #[deprecated(
        since = "0.9.0",
        note = "with `generic-snapshot-data` enabled, use `full_snapshot()` instead to send full snapshot"
    )]
    async fn install_snapshot(
        &mut self,
        _rpc: crate::raft::InstallSnapshotRequest<C>,
        _option: RPCOption,
    ) -> Result<
        crate::raft::InstallSnapshotResponse<C::NodeId>,
        RPCError<C::NodeId, C::Node, RaftError<C::NodeId, crate::error::InstallSnapshotError>>,
    > {
        unimplemented!()
    }

    /// Send an InstallSnapshot RPC to the target.
    #[cfg(not(feature = "generic-snapshot-data"))]
    async fn install_snapshot(
        &mut self,
        _rpc: crate::raft::InstallSnapshotRequest<C>,
        _option: RPCOption,
    ) -> Result<
        crate::raft::InstallSnapshotResponse<C::NodeId>,
        RPCError<C::NodeId, C::Node, RaftError<C::NodeId, crate::error::InstallSnapshotError>>,
    >;

    /// Send a RequestVote RPC to the target.
    async fn vote(
        &mut self,
        rpc: VoteRequest<C::NodeId>,
        option: RPCOption,
    ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>>;

    /// Send a complete Snapshot to the target.
    ///
    /// This method is responsible to fragment the snapshot and send it to the target node.
    /// Before returning from this method, the snapshot should be completely transmitted and
    /// installed on the target node, or rejected because of `vote` being smaller than the
    /// remote one.
    ///
    /// The default implementation just calls several `install_snapshot` RPC for each fragment.
    ///
    /// The `vote` is the leader vote which is used to check if the leader is still valid by a
    /// follower.
    /// When the follower finished receiving snapshot, it calls `Raft::install_full_snapshot()`
    /// with this vote.
    ///
    /// `cancel` get `Ready` when the caller decides to cancel this snapshot transmission.
    ///
    /// To implement a more generic snapshot transmission, you can use the `generic-snapshot-data`
    /// feature. Enabling this feature allows you to send any type of snapshot data.
    /// See the [generic snapshot
    /// data](crate::docs::feature_flags#feature-flag-generic-snapshot-data) chapter for
    /// details.
    #[cfg(feature = "generic-snapshot-data")]
    async fn full_snapshot(
        &mut self,
        vote: Vote<C::NodeId>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>>;

    /// Send a complete Snapshot to the target.
    ///
    /// This method is responsible to fragment the snapshot and send it to the target node.
    /// Before returning from this method, the snapshot should be completely transmitted and
    /// installed on the target node, or rejected because of `vote` being smaller than the
    /// remote one.
    ///
    /// The default implementation just calls several `install_snapshot` RPC for each fragment.
    ///
    /// The `vote` is the leader vote which is used to check if the leader is still valid by a
    /// follower.
    /// When the follower finished receiving snapshot, it calls `Raft::install_full_snapshot()`
    /// with this vote.
    ///
    /// `cancel` get `Ready` when the caller decides to cancel this snapshot transmission.
    ///
    /// To implement a more generic snapshot transmission, you can use the `generic-snapshot-data`
    /// feature. Enabling this feature allows you to send any type of snapshot data.
    /// See the [generic snapshot
    /// data](crate::docs::feature_flags#feature-flag-generic-snapshot-data) chapter for
    /// details.
    // If generic-snapshot-data disabled,
    // provide a default implementation that relies on AsyncRead + AsyncSeek + Unpin
    #[cfg(not(feature = "generic-snapshot-data"))]
    async fn full_snapshot(
        &mut self,
        vote: Vote<C::NodeId>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
        use crate::network::snapshot_transport::Chunked;
        use crate::network::snapshot_transport::SnapshotTransport;

        let resp = Chunked::send_snapshot(self, vote, snapshot, cancel, option).await?;
        Ok(resp)
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
