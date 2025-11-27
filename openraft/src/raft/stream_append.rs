//! Stream-based AppendEntries API implementation with pipelining.

use std::sync::Arc;

use futures::Stream;
use futures::StreamExt;

use crate::AsyncRuntime;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::core::raft_msg::RaftMsg;
use crate::entry::RaftEntry;
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
    prev_log_id: Option<LogIdOf<C>>,
    last_log_id: Option<LogIdOf<C>>,
}

/// Create a pipelined stream that processes AppendEntries requests.
///
/// Spawns a background task that reads from input, sends to RaftCore,
/// and forwards response receivers. The returned stream awaits responses in order.
pub(crate) fn stream_append<C, S>(
    inner: Arc<RaftInner<C>>,
    input: S,
) -> impl Stream<Item = StreamAppendResult<C>> + OptionalSend + 'static
where
    C: RaftTypeConfig,
    S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + 'static,
{
    let (tx, rx) = C::mpsc::<Pending<C>>(PIPELINE_BUFFER_SIZE);

    let inner2 = inner.clone();
    let _handle = C::AsyncRuntime::spawn(async move {
        let inner = inner2;
        futures::pin_mut!(input);

        while let Some(req) = input.next().await {
            let prev = req.prev_log_id.clone();
            let last = req.entries.last().map(|e| e.log_id()).or(prev.clone());
            let (resp_tx, resp_rx) = C::oneshot();

            if inner.send_msg(RaftMsg::AppendEntries { rpc: req, tx: resp_tx }).await.is_err() {
                break;
            }
            let pending = Pending {
                response_rx: resp_rx,
                prev_log_id: prev,
                last_log_id: last,
            };
            if MpscSender::send(&tx, pending).await.is_err() {
                break;
            }
        }
    });

    futures::stream::unfold(Some((rx, inner)), |state| async move {
        let (mut rx, inner) = state?;
        let p: Pending<C> = MpscReceiver::recv(&mut rx).await?;

        let resp = inner.recv_msg(p.response_rx).await.ok()?;

        let result = resp.into_stream_result(p.prev_log_id, p.last_log_id);
        let cont = result.is_ok();

        Some((result, if cont { Some((rx, inner)) } else { None }))
    })
}
