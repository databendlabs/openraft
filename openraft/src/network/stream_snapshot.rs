use std::future::Future;
use std::io::SeekFrom;
use std::pin::Pin;
use std::time::Duration;

use futures::FutureExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;

use crate::error::Fatal;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::network::RPCOption;
use crate::raft::InstallSnapshotRequest;
use crate::raft::SnapshotResponse;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::AsyncRuntime;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::RaftNetwork;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::ToStorageResult;
use crate::Vote;

/// Stream snapshot by chunks.
///
/// This function is for backward compatibility and provides a default implement for
/// `RaftNetwork::snapshot()` upon `RafNetwork::install_snapshot()`, which requires `SnapshotData`
/// to be `AsyncRead + AsyncSeek`.
pub(crate) async fn stream_snapshot<C, Net>(
    net: &mut Net,
    vote: Vote<C::NodeId>,
    mut snapshot: Snapshot<C>,
    mut cancel: impl Future<Output = ReplicationClosed>,
    option: RPCOption,
) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>>
where
    C: RaftTypeConfig,
    Net: RaftNetwork<C> + ?Sized,
{
    let subject_verb = || (ErrorSubject::Snapshot(Some(snapshot.meta.signature())), ErrorVerb::Read);

    let mut offset = 0;
    let end = snapshot.snapshot.seek(SeekFrom::End(0)).await.sto_res(subject_verb)?;

    loop {
        // Safety: `cancel` is a future that is polled only by this function.
        let c = unsafe { Pin::new_unchecked(&mut cancel) };

        // If canceled, return at once
        if let Some(err) = c.now_or_never() {
            return Err(err.into());
        }

        // Sleep a short time otherwise in test environment it is a dead-loop that never
        // yields.
        // Because network implementation does not yield.
        AsyncRuntimeOf::<C>::sleep(Duration::from_millis(10)).await;

        snapshot.snapshot.seek(SeekFrom::Start(offset)).await.sto_res(subject_verb)?;

        // Safe unwrap(): this function is called only by default implementation of
        // `RaftNetwork::snapshot()` and it is always set.
        let chunk_size = option.snapshot_chunk_size().unwrap();
        let mut buf = Vec::with_capacity(chunk_size);
        while buf.capacity() > buf.len() {
            let n = snapshot.snapshot.read_buf(&mut buf).await.sto_res(subject_verb)?;
            if n == 0 {
                break;
            }
        }

        let n_read = buf.len();

        let done = (offset + n_read as u64) == end;
        let req = InstallSnapshotRequest {
            vote,
            meta: snapshot.meta.clone(),
            offset,
            data: buf,
            done,
        };

        // Send the RPC over to the target.
        tracing::debug!(
            snapshot_size = req.data.len(),
            req.offset,
            end,
            req.done,
            "sending snapshot chunk"
        );

        #[allow(deprecated)]
        let res = AsyncRuntimeOf::<C>::timeout(option.hard_ttl(), net.install_snapshot(req, option.clone())).await;

        let resp = match res {
            Ok(outer_res) => match outer_res {
                Ok(res) => res,
                Err(err) => {
                    tracing::warn!(error=%err, "error sending InstallSnapshot RPC to target");
                    continue;
                }
            },
            Err(err) => {
                tracing::warn!(error=%err, "timeout while sending InstallSnapshot RPC to target");
                continue;
            }
        };

        if resp.vote > vote {
            // Unfinished, return a response with a higher vote.
            // The caller checks the vote and return a HigherVote error.
            return Ok(SnapshotResponse::new(resp.vote));
        }

        if done {
            return Ok(SnapshotResponse::new(resp.vote));
        }

        offset += n_read as u64;
    }
}
