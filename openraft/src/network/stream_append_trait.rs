//! Defines the [`NetStreamAppend`] trait for streaming AppendEntries.

use display_more::DisplayOptionExt;
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
use crate::raft::AppendResult;

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
pub trait NetStreamAppend<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
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
    /// A conflict may carry the hint that [`Raft::stream_append()`] returned with it, which lets
    /// the leader skip part of its binary search. A transport that cannot carry the hint returns
    /// the conflict without it.
    ///
    /// The output stream terminates when the input is exhausted or an error occurs.
    /// The network implementation is responsible for enforcing `option.soft_ttl()`.
    ///
    /// `option.hard_ttl()` is not a hard limit on the lifetime of a long-lived stream. Streaming
    /// transports should use `soft_ttl()` for setup, per-request-response, or idle-timeout policy,
    /// and return an error when the connection stops making progress.
    ///
    /// # Cancellation
    ///
    /// The caller may drop the returned future before it resolves, and the output stream before
    /// it ends. Openraft does so in these cases:
    ///
    /// - Log replication drops the output stream, without polling it again, as soon as the stream
    ///   yields an error, a conflict, or a higher vote. It also drops the stream when replication
    ///   to the target stops, such as when this node is no longer the leader.
    /// - The heartbeat sender, whose streams carry a single request each, drops the returned future
    ///   or the output stream once `option.hard_ttl()` has passed since the call, or as soon as the
    ///   stream yields an error. It also drops them when heartbeats to the target stop.
    ///
    /// A dropped future or stream does not resume: only its destructors run, so cleanup placed
    /// after an `.await`, or left for a later poll of the output, never runs. Work that this method
    /// moved to another task, such as a task that reads `input` and sends the requests, outlives
    /// the drop unless a value that the returned future or stream owns stops it, such as a guard
    /// that cancels the task when dropped, and the task must notice the stop even while it waits
    /// for room to send a request. Openraft also ends `input` when it drops the future or the
    /// stream: the input of log replication yields `None` then, or once a log read that it has
    /// already started finishes, and the input of a heartbeat holds a single request. A task that
    /// waits for the next request from `input` thus sees the input end.
    ///
    /// The stop does not have to be immediate: the task may finish a send that it has already
    /// started, and the drop need not wait for the task to end. The transport need not undo
    /// requests it sent before the drop: the follower may still process them.
    ///
    /// Openraft may then call `stream_append()` again on the same `self`: log replication for its
    /// next stream, and the heartbeat sender for its next heartbeat. The output stream borrows
    /// `self`, so that call starts only after the drop, but a task of the dropped stream may still
    /// be finishing then. Keep `self` safe for the call:
    ///
    /// - A dropped stream may leave a request partly written, or responses unread, on a connection
    ///   that `self` keeps for later calls. Discard such a connection instead of reusing it as it
    ///   is.
    /// - Never yield a response to a request of a dropped stream from a later stream: each response
    ///   must come from a request derived from the stream's own input, as described above.
    ///
    /// # Note
    ///
    /// This method returns `BoxFuture` and `BoxStream` instead of `impl Future`/`impl Stream`
    /// to avoid a higher-ranked lifetime error that occurs when the return type
    /// captures the lifetime `'s` in an `impl Trait` position.
    ///
    /// [`Raft::stream_append()`]: crate::raft::Raft::stream_append
    #[since(
        version = "0.10.0",
        change = "response stream yields AppendError with an optional conflict hint"
    )]
    fn stream_append<'s, S>(
        &'s mut self,
        input: S,
        option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<AppendResult<C>, RPCError<C>>>, RPCError<C>>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static;
}

/// Default sequential implementation of stream_append.
///
/// This processes requests one at a time: send request, wait for response, repeat. A partial
/// acknowledgement or a rejection ends the stream.
///
/// The helper starts no task: dropping the returned future or stream drops `input` and the
/// [`NetAppend::append_entries()`] call in progress, if any. It meets the cancellation rules of
/// [`NetStreamAppend::stream_append`] when a dropped `append_entries()` call leaves `network` safe
/// for the next call.
#[since(version = "0.10.0", change = "response stream yields AppendError")]
pub fn stream_append_sequential<'s, C, N, S>(
    network: &'s mut N,
    input: S,
    option: RPCOption,
) -> BoxFuture<'s, Result<BoxStream<'s, Result<AppendResult<C>, RPCError<C>>>, RPCError<C>>>
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
                    Ok(append_result) => {
                        if let Ok(acked) = &append_result {
                            debug_assert!(
                                range.prev <= *acked && *acked <= range.last,
                                "acknowledged {} outside the sent range {}",
                                acked.display(),
                                range
                            );
                        }

                        // Only an acknowledgement of the whole request lets the stream go on.
                        let fully_acked = append_result.as_ref().ok() == Some(&range.last);
                        let next_state = if fully_acked { Some((network, input)) } else { None };
                        Some((Ok(append_result), next_state))
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
