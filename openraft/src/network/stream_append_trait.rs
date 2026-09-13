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
use crate::raft::StreamAppendResult;
use crate::raft::StreamAppendSuccess;

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
    /// The output stream terminates when the input is exhausted, an error occurs, or a strict
    /// [`StreamAppendSuccess::Partial`] is returned. A partial success must be the last output item
    /// so Openraft can resume replication from its matching log id in a new stream.
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
    #[since(version = "0.10.0", change = "stream success distinguishes full and partial")]
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
#[since(version = "0.10.0", change = "stream success distinguishes full and partial")]
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
                        let stream_result = resp.into_stream_result(range.prev, range.last.clone());
                        let next_state = match &stream_result {
                            Ok(StreamAppendSuccess::Full(_)) => Some((network, input)),
                            Ok(StreamAppendSuccess::Partial(_)) | Err(_) => None,
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
