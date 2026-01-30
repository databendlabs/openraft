//! Stream-based AppendEntries API implementation with pipelining.

use std::sync::Arc;

use futures_util::Stream;
use futures_util::StreamExt;

use crate::AsyncRuntime;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::core::raft_msg::RaftMsg;
use crate::log_id_range::LogIdRange;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::StreamAppendError;
use crate::raft::raft_inner::RaftInner;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::async_runtime::MpscReceiver;
use crate::type_config::async_runtime::MpscSender;
use crate::type_config::util::TypeConfigExt;

/// Result type for stream append operations.
pub type StreamAppendResult<C> = Result<Option<LogIdOf<C>>, StreamAppendError<C>>;

const PIPELINE_BUFFER_SIZE: usize = 64;

struct Pending<C: RaftTypeConfig> {
    response_rx: OneshotReceiverOf<C, AppendEntriesResponse<C>>,
    log_id_range: LogIdRange<C>,
}

/// Create a pipelined stream that processes AppendEntries requests.
///
/// Spawns a background task that reads from input, sends to RaftCore,
/// and forwards response receivers. The returned stream awaits responses in order.
///
/// On error (Conflict or HigherVote), the stream terminates immediately.
/// The background task exits when it fails to send to the dropped channel.
pub(crate) fn stream_append<C, S>(
    inner: Arc<RaftInner<C>>,
    input: S,
) -> impl Stream<Item = StreamAppendResult<C>> + OptionalSend + 'static
where
    C: RaftTypeConfig,
    S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + 'static,
{
    let (tx, rx) = C::mpsc::<Pending<C>>(PIPELINE_BUFFER_SIZE);

    let inner_clone = inner.clone();

    let _join_handle = C::AsyncRuntime::spawn(async move {
        futures_util::pin_mut!(input);

        while let Some(req) = input.next().await {
            let log_id_range = req.log_id_range();
            let (resp_tx, resp_rx) = C::oneshot();

            if inner_clone.send_msg(RaftMsg::AppendEntries { rpc: req, tx: resp_tx }).await.is_err() {
                break;
            }

            let pending = Pending {
                response_rx: resp_rx,
                log_id_range,
            };

            if MpscSender::send(&tx, pending).await.is_err() {
                break;
            }
        }
    });

    futures_util::stream::unfold(Some((rx, inner)), |state| async move {
        let (mut rx, inner) = state?;
        let p: Pending<C> = MpscReceiver::recv(&mut rx).await?;

        let resp = inner.recv_msg(p.response_rx).await.ok()?;
        let range = p.log_id_range;
        let result = resp.into_stream_result(range.prev, range.last);

        if result.is_err() {
            return Some((result, None));
        }

        Some((result, Some((rx, inner))))
    })
}
