use std::future::Future;
use std::time::Duration;

use anyerror::AnyError;
use futures::Stream;
use futures::StreamExt;
use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::base::BoxFuture;
use crate::base::BoxStream;
use crate::error::RPCError;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::error::Unreachable;
use crate::network::Backoff;
use crate::network::RPCOption;
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
        let fu = async move {
            let strm = futures::stream::unfold(Some((self, input)), move |state| {
                let option = option.clone();
                async move {
                    let (network, mut input) = state?;

                    let req = input.next().await?;

                    let range = req.log_id_range();

                    let result = network.append_entries(req, option).await;

                    match result {
                        Ok(resp) => {
                            let partial_success = resp.get_partial_success().cloned();

                            let stream_result = resp.into_stream_result(range.prev, range.last.clone());
                            let is_err = stream_result.is_err();
                            let next_state = if is_err {
                                None
                            } else if let Some(partial) = partial_success {
                                if partial == range.last {
                                    Some((network, input))
                                } else {
                                    // If the request is only partially finished, pipeline should be stopped
                                    None
                                }
                            } else {
                                // full success
                                Some((network, input))
                            };
                            Some((Ok(stream_result), next_state))
                        }
                        Err(e) => Some((Err(e), None)),
                    }
                }
            });

            let strm: BoxStream<'s, _> = Box::pin(strm);
            Ok(strm)
        };

        Box::pin(fu)
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

use crate::network::RaftNetworkBackoff;
use crate::network::RaftNetworkSnapshot;
use crate::network::RaftNetworkStreamAppend;
use crate::network::RaftNetworkTransferLeader;
use crate::network::RaftNetworkVote;

impl<C, T> RaftNetworkBackoff<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C>,
{
    fn backoff(&self) -> Backoff {
        RaftNetworkV2::backoff(self)
    }
}

#[allow(clippy::manual_async_fn)]
impl<C, T> RaftNetworkVote<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C>,
{
    async fn vote(&mut self, rpc: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        RaftNetworkV2::vote(self, rpc, option).await
    }
}

#[allow(clippy::manual_async_fn)]
impl<C, T> RaftNetworkSnapshot<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C>,
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
impl<C, T> RaftNetworkTransferLeader<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C>,
{
    async fn transfer_leader(
        &mut self,
        req: TransferLeaderRequest<C>,
        option: RPCOption,
    ) -> Result<(), RPCError<C>> {
        RaftNetworkV2::transfer_leader(self, req, option).await
    }
}

impl<C, T> RaftNetworkStreamAppend<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkV2<C>,
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
