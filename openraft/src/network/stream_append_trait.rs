//! Defines the [`NetStreamAppend`] trait for streaming AppendEntries.

use futures_util::Stream;
use futures_util::StreamExt;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::base::BoxFuture;
use crate::base::BoxStream;
use crate::error::RPCError;
use crate::network::NetAppend;
use crate::network::RPCOption;
use crate::raft::AppendEntriesRequest;
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
pub trait NetStreamAppend<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send a stream of AppendEntries RPCs to the target and return a stream of responses.
    ///
    /// This method forwards a stream of AppendEntries requests to the remote follower.
    /// The remote follower should call [`Raft::stream_append()`] to process the stream
    /// and send back a stream of responses.
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
