//! Defines the [`NetStreamAppend`] trait for streaming AppendEntries.

use futures_util::Stream;
use futures_util::StreamExt;
use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::base::BoxFuture;
use crate::base::BoxStream;
use crate::errors::RPCError;
use crate::network::NetAppend;
use crate::network::RPCOption;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponseWithHint;
use crate::raft::StreamAppendResult;

/// Sends a stream of AppendEntries RPCs to a target node.
///
/// This trait provides streaming capabilities for AppendEntries.
///
/// **For most applications, implement [`RaftNetworkV2`] instead.** This trait is
/// automatically derived from `RaftNetworkV2` via blanket implementation.
///
/// Direct implementation is useful for advanced cases like:
/// - Native gRPC bidirectional streaming
/// - Custom pipelining strategies
/// - Protocol-specific optimizations
///
/// This trait can be obtained via:
/// 1. **From RaftNetworkV2**: Blanket impl delegates to `RaftNetworkV2::stream_append()`
/// 2. **Direct implementation**: Implement the trait directly
///
/// For implementations that want to build streaming on top of single-request
/// [`NetAppend`], use [`stream_append_sequential`] helper.
///
/// [`RaftNetworkV2`]: crate::network::RaftNetworkV2
#[since(version = "0.10.0", change = "added optional append_entries_with_hint")]
pub trait NetStreamAppend<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Optionally send one AppendEntries probe through an endpoint that returns conflict hints.
    ///
    /// Return `None` only when this endpoint is unsupported and the request was not processed.
    /// OpenRaft then sends the request through [`Self::stream_append`]. The default returns
    /// `None` without sending anything, preserving existing transports and wire formats.
    /// Implementations should call
    /// [`Raft::append_entries_with_hint`](crate::Raft::append_entries_with_hint) remotely and
    /// apply `option.soft_ttl()` to the RPC.
    #[since(version = "0.10.0")]
    fn append_entries_with_hint(
        &mut self,
        _rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> BoxFuture<'_, Result<Option<AppendEntriesResponseWithHint<C>>, RPCError<C>>> {
        Box::pin(async { Ok(None) })
    }

    /// Send a stream of AppendEntries RPCs to the target and return a stream of responses.
    ///
    /// This method forwards a stream of AppendEntries requests to the remote follower.
    /// The remote follower should call [`Raft::stream_append()`] to process the stream
    /// and send back a stream of responses.
    ///
    /// A transport may merge or split input requests, including sending entry-less requests at
    /// segment boundaries. It must preserve log order, the leader vote, and commit semantics.
    /// The number of responses need not equal the number of input requests. Responses must follow
    /// the order of the requests actually sent; a pipelined transport must reorder concurrent
    /// results before yielding them.
    ///
    /// Each response must come from the follower processing a request derived from this input
    /// stream. Full success reports that request's `last_log_id`; partial success reports a
    /// matching log id within its inclusive `[prev_log_id, last_log_id]` range. A conflict reports
    /// the `prev_log_id` of the request the follower rejected. These bounds also apply to merged,
    /// split, and entry-less requests; a response must not claim log progress beyond the consumed
    /// input.
    ///
    /// The output stream terminates when the input is exhausted or an error occurs.
    /// The network implementation is responsible for enforcing `option.soft_ttl()`.
    ///
    /// `option.hard_ttl()` is not a hard limit on the lifetime of a long-lived stream. Streaming
    /// transports should use `soft_ttl()` for setup, per-request-response, or idle-timeout policy,
    /// and return an error when the connection stops making progress.
    ///
    /// # Note
    ///
    /// This method returns `BoxFuture` and `BoxStream` instead of `impl Future`/`impl Stream`
    /// to avoid a higher-ranked lifetime error that occurs when the return type
    /// captures the lifetime `'s` in an `impl Trait` position.
    ///
    /// [`Raft::stream_append()`]: crate::raft::Raft::stream_append
    fn stream_append<'s, S>(
        &'s mut self,
        input: S,
        option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<StreamAppendResult<C>, RPCError<C>>>, RPCError<C>>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static;
}

/// Default sequential implementation of stream_append.
///
/// This processes requests one at a time: send request, wait for response, repeat.
pub fn stream_append_sequential<'s, C, N, S>(
    network: &'s mut N,
    input: S,
    option: RPCOption,
) -> BoxFuture<'s, Result<BoxStream<'s, Result<StreamAppendResult<C>, RPCError<C>>>, RPCError<C>>>
where
    C: RaftTypeConfig,
    N: NetAppend<C> + ?Sized,
    S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
{
    let fu = async move {
        let strm = futures_util::stream::unfold(Some((network, input)), move |state| {
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
