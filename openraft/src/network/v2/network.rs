use std::future::Future;

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
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::VoteOf;

/// A trait defining the interface for a Raft network between cluster members.
///
/// **This is the recommended trait for most applications.** It provides a unified interface
/// for all network operations with sensible defaults. Simply implement the required methods
/// (`append_entries`, `vote`, `full_snapshot`) and the sub-traits ([`NetAppend`], [`NetVote`],
/// [`NetSnapshot`], etc.) will be automatically derived via blanket implementations.
///
/// # Design: unary RPCs with streaming compatibility
///
/// `RaftNetworkV2` is designed around **request-response (unary) RPCs** — `append_entries`,
/// `vote`, and `full_snapshot` each send one request and produce one response. When this
/// trait was introduced, it was not designed for stream-oriented AppendEntries.
///
/// It remains **compatible** with the stream-oriented API, however: if an application only
/// implements the unary `append_entries`, Openraft adapts it into a stream by calling it
/// sequentially via the default [`stream_append`](Self::stream_append) implementation
/// (see [`stream_append_sequential`]). This is
/// convenient but not optimal for throughput.
///
/// For true streaming performance while keeping the unified interface, an application can
/// **override the default [`stream_append`](Self::stream_append)** on `RaftNetworkV2` itself
/// with a custom implementation (e.g. native gRPC bidirectional streaming or pipelined
/// AppendEntries) — without giving up the convenience of `RaftNetworkV2` for the other RPCs.
///
/// # Implementing sub-traits directly for optimal performance
///
/// For best performance — for example, native gRPC bidirectional streaming for
/// AppendEntries — implement the individual sub-traits directly instead of `RaftNetworkV2`.
/// These are the exact bounds required by [`RaftNetworkFactory::Network`], so any
/// combination of direct impls satisfies the factory:
///
/// - [`NetAppend`] — unary AppendEntries
/// - [`NetVote`] — RequestVote
/// - [`NetSnapshot`] — full snapshot transfer
/// - [`NetStreamAppend`] — stream-oriented AppendEntries (implement directly for native gRPC bidi
///   streaming or pipelining)
/// - [`NetTransferLeader`] — TransferLeader notification
/// - [`NetBackoff`] — backoff strategy on `Unreachable`
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
/// [`NetTransferLeader`]: crate::network::NetTransferLeader
/// [`NetBackoff`]: crate::network::NetBackoff
/// [`RaftNetworkFactory::Network`]: crate::network::RaftNetworkFactory::Network
/// [correct-node]: `crate::docs::cluster_control::dynamic_membership#ensure-connection-to-the-correct-node`
#[since(version = "0.10.0")]
#[add_async_trait]
pub trait RaftNetworkV2<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Snapshot data this network implementation can transmit.
    #[since(
        version = "0.10.0",
        change = "moved SnapshotData from RaftTypeConfig to RaftNetworkV2"
    )]
    type SnapshotData: OptionalSend + 'static;

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

    /// Send a Pre-Vote RPC to the target.
    ///
    /// The node receiving this message should pass it to [`Raft::pre_vote()`]. Pre-Vote asks the
    /// target whether it *would* grant a vote for `rpc.vote` (a hypothetical `term + 1`) without
    /// persisting any vote or changing its term. It is only sent when
    /// [`Config::enable_pre_vote`](crate::Config::enable_pre_vote) is set.
    ///
    /// The default implementation synthesizes a **granting** response, so a network that has not
    /// implemented `pre_vote` makes Pre-Vote a no-op and elections proceed exactly as before (e.g.
    /// during a rolling upgrade). This is deliberately distinct from a transport failure: an
    /// implementor that cannot reach the target must return `Err` (typically
    /// [`Unreachable`]), which the caller does **not** count as a grant — otherwise a fully
    /// isolated node would synthesize a quorum of grants and inflate its term, defeating
    /// Pre-Vote.
    ///
    /// [`Raft::pre_vote()`]: crate::raft::Raft::pre_vote
    #[since(version = "0.10.0", change = "added pre_vote RPC for the Pre-Vote feature")]
    async fn pre_vote(&mut self, rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        // Not implemented: grant unconditionally so Pre-Vote degrades to the normal election path.
        Ok(VoteResponse::new(rpc.vote, None, true))
    }

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
        snapshot: SnapshotOf<C, Self::SnapshotData>,
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
    /// Return `None` (the default) to use the backoff configured in
    /// [`Config::backoff`](crate::Config::backoff).
    #[since(
        version = "0.10.0",
        change = "changed return type to Option<Backoff>; None delegates to Config::backoff"
    )]
    fn backoff(&self) -> Option<Backoff> {
        None
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
    fn backoff(&self) -> Option<Backoff> {
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

    async fn pre_vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        RaftNetworkV2::pre_vote(self, rpc, option).await
    }
}

#[allow(clippy::manual_async_fn)]
impl<C, T> NetSnapshot<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C> + ?Sized,
{
    type SnapshotData = T::SnapshotData;

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: SnapshotOf<C, Self::SnapshotData>,
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
