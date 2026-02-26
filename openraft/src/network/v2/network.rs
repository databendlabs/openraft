use std::future::Future;
use std::time::Duration;

use anyerror::AnyError;
use futures_util::Stream;
use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::base::BoxFuture;
use crate::base::BoxStream;
use crate::errors::RPCError;
use crate::errors::ReplicationClosed;
use crate::errors::StreamingError;
use crate::errors::Unreachable;
use crate::network::Backoff;
use crate::network::NetAppend;
use crate::network::RPCOption;
use crate::network::stream_append_sequential;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::SnapshotResponse;
use crate::raft::StreamAppendResult;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::message::TransferLeaderRequest;
use crate::storage::Snapshot;
use crate::type_config::alias::VoteOf;

/// A trait defining the interface for a Raft network between cluster members.
///
/// **This is the recommended trait for most applications.** It provides a unified interface
/// for all network operations with sensible defaults. Simply implement the required methods
/// (`append_entries`, `vote`, `full_snapshot`) and the sub-traits ([`NetAppend`], [`NetVote`],
/// [`NetSnapshot`], etc.) will be automatically derived via blanket implementations.
///
/// For advanced use cases requiring fine-grained control, applications may implement
/// the individual sub-traits directly instead of this trait. See [`NetStreamAppend`] for
/// an example where direct implementation enables native gRPC bidirectional streaming.
///
/// See the [network chapter of the guide](crate::docs::getting_started#4-implement-raftnetwork)
/// for details and discussion on this trait and how to implement it.
///
/// A single network instance is used to connect to a single target node. The network instance is
/// constructed by the [`RaftNetworkFactory`](`crate::network::RaftNetworkFactory`).
///
/// V2 network API removes the `install_snapshot()` method that sends snapshot in chunks
/// and introduces the `full_snapshot()` method that lets the application fully customize snapshot
/// transmit.
///
/// [Ensure connection to correct node][correct-node]
///
/// [`NetAppend`]: crate::network::NetAppend
/// [`NetVote`]: crate::network::NetVote
/// [`NetSnapshot`]: crate::network::NetSnapshot
/// [`NetStreamAppend`]: crate::network::NetStreamAppend
/// [correct-node]: `crate::docs::cluster_control::dynamic_membership#ensure-connection-to-the-correct-node`
#[since(version = "0.10.0")]
#[add_async_trait]
pub trait RaftNetworkV2<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send an AppendEntries RPC to the target.
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>>;

    /// Send a stream of AppendEntries RPCs to the target and return a stream of responses.
    ///
    /// This method forwards a stream of AppendEntries requests to the remote follower.
    /// The remote follower should call [`Raft::stream_append()`] to process the stream
    /// and send back a stream of responses.
    ///
    /// The default implementation processes requests sequentially in a request-response
    /// manner: it sends one request, waits for the response, then sends the next.
    /// This is simple but not optimal for performance.
    ///
    /// Since network delivery order is not guaranteed, the default implementation
    /// does not attempt pipelining. Applications requiring higher throughput should
    /// override this method with a custom implementation that handles out-of-order
    /// delivery appropriately (e.g., using sequence numbers or a reliable transport).
    ///
    /// The output stream terminates when the input is exhausted or an error occurs.
    ///
    /// # Note
    ///
    /// This method returns `BoxFuture` and `BoxStream` instead of `impl Future`/`impl Stream`
    /// to avoid a higher-ranked lifetime error that occurs when the return type
    /// captures the lifetime `'s` in an `impl Trait` position.
    ///
    /// [`Raft::stream_append()`]: crate::raft::Raft::stream_append
    #[since(version = "0.10.0")]
    fn stream_append<'s, S>(
        &'s mut self,
        input: S,
        option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<StreamAppendResult<C>, RPCError<C>>>, RPCError<C>>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
    {
        stream_append_sequential(self, input, option)
    }

    /// Send a RequestVote RPC to the target.
    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>>;

    /// Send a complete Snapshot to the target.
    ///
    /// This method is responsible for fragmenting the snapshot and sending it to the target node.
    /// Before returning from this method, the snapshot should be completely transmitted and
    /// installed on the target node or rejected because of `vote` being smaller than the
    /// remote one.
    ///
    /// The default implementation just calls several `install_snapshot` RPCs for each fragment.
    ///
    /// The `vote` is the leader vote used to check if the leader is still valid by a
    /// follower.
    /// When the follower finished receiving the snapshot, it calls
    /// [`Raft::install_full_snapshot()`] with this vote.
    ///
    /// `cancel` gets `Ready` when the caller decides to cancel this snapshot transmission.
    ///
    /// [`Raft::install_full_snapshot()`]: crate::raft::Raft::install_full_snapshot
    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>>;

    /// Send TransferLeader message to the target node.
    ///
    /// The node received this message should pass it to [`Raft::handle_transfer_leader()`].
    ///
    /// This method provides a default implementation that just returns [`Unreachable`] error to
    /// ignore it. In case the application did not implement it, other nodes just wait for the
    /// Leader lease to timeout and then restart election.
    ///
    /// [`Raft::handle_transfer_leader()`]: crate::raft::Raft::handle_transfer_leader
    #[since(version = "0.10.0")]
    async fn transfer_leader(&mut self, _req: TransferLeaderRequest<C>, _option: RPCOption) -> Result<(), RPCError<C>> {
        Err(RPCError::Unreachable(Unreachable::new(&AnyError::error(
            "transfer_leader not implemented",
        ))))
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
    /// By default, it returns a constant backoff of 500 ms.
    fn backoff(&self) -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(200)))
    }
}

// =============================================================================
// Blanket implementations: RaftNetworkV2 → Sub-traits
// =============================================================================
//
// These blanket impls allow existing RaftNetworkV2 implementations to
// automatically satisfy all sub-trait requirements by delegating to
// the corresponding RaftNetworkV2 methods.

use crate::network::NetBackoff;
use crate::network::NetSnapshot;
use crate::network::NetStreamAppend;
use crate::network::NetTransferLeader;
use crate::network::NetVote;

#[allow(clippy::manual_async_fn)]
impl<C, T> NetAppend<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C> + ?Sized,
{
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        RaftNetworkV2::append_entries(self, rpc, option).await
    }
}

impl<C, T> NetBackoff<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C> + ?Sized,
{
    fn backoff(&self) -> Backoff {
        RaftNetworkV2::backoff(self)
    }
}

#[allow(clippy::manual_async_fn)]
impl<C, T> NetVote<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C> + ?Sized,
{
    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        RaftNetworkV2::vote(self, rpc, option).await
    }
}

#[allow(clippy::manual_async_fn)]
impl<C, T> NetSnapshot<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C> + ?Sized,
{
    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        RaftNetworkV2::full_snapshot(self, vote, snapshot, cancel, option).await
    }
}

#[allow(clippy::manual_async_fn)]
impl<C, T> NetTransferLeader<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C> + ?Sized,
{
    async fn transfer_leader(&mut self, req: TransferLeaderRequest<C>, option: RPCOption) -> Result<(), RPCError<C>> {
        RaftNetworkV2::transfer_leader(self, req, option).await
    }
}

impl<C, T> NetStreamAppend<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C> + ?Sized,
{
    fn stream_append<'s, S>(
        &'s mut self,
        input: S,
        option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<StreamAppendResult<C>, RPCError<C>>>, RPCError<C>>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
    {
        RaftNetworkV2::stream_append(self, input, option)
    }
}
