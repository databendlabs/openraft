use std::future::Future;
use std::io::SeekFrom;
use std::time::Duration;

use futures::FutureExt;
use macros::add_async_trait;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

use crate::error::Fatal;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::network::streaming::Streaming;
use crate::network::RPCOption;
use crate::raft::InstallSnapshotRequest;
use crate::raft::SnapshotResponse;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::AsyncRuntime;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::OptionalSend;
use crate::RaftNetwork;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::StorageError;
use crate::StorageIOError;
use crate::ToStorageResult;
use crate::Vote;

#[add_async_trait]
pub(crate) trait SnapshotTransport<C: RaftTypeConfig> {
    async fn send_snapshot<Net>(
        _net: &mut Net,
        _vote: Vote<C::NodeId>,
        _snapshot: Snapshot<C>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>>
    where
        Net: RaftNetwork<C> + ?Sized,
    {
        unimplemented!("send_snapshot is only implemented with SnapshotData with AsyncRead + AsyncSeek ...")
    }

    async fn receive_snapshot(
        _streaming: &mut Option<Streaming<C>>,
        _req: InstallSnapshotRequest<C>,
    ) -> Result<Option<Snapshot<C>>, StorageError<C::NodeId>> {
        unimplemented!("receive_snapshot is only implemented with SnapshotData with AsyncWrite + AsyncSeek ...")
    }
}

/// Send and Receive snapshot by chunks.
pub(crate) struct Chunked {}

impl<C: RaftTypeConfig> SnapshotTransport<C> for Chunked {
    /// Stream snapshot by chunks.
    ///
    /// This function is for backward compatibility and provides a default implement for
    /// `RaftNetwork::snapshot()` upon `RafNetwork::install_snapshot()`. This implementation
    /// requires `SnapshotData` to be `AsyncRead + AsyncSeek`.
    ///
    /// The argument `vote` is the leader's vote which is used to check if the leader is still valid
    /// by a follower.
    ///
    /// `cancel` is a future that is polled only by this function. It return Ready if the caller
    /// decide to cancel this snapshot transmission.
    async fn send_snapshot<Net>(
        net: &mut Net,
        vote: Vote<C::NodeId>,
        mut snapshot: Snapshot<C>,
        mut cancel: impl Future<Output = ReplicationClosed> + OptionalSend,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>>
    where
        Net: RaftNetwork<C> + ?Sized,
    {
        let subject_verb = || (ErrorSubject::Snapshot(Some(snapshot.meta.signature())), ErrorVerb::Read);

        let mut offset = 0;
        let end = snapshot.snapshot.seek(SeekFrom::End(0)).await.sto_res(subject_verb)?;

        let mut c = std::pin::pin!(cancel);
        loop {
            // If canceled, return at once
            if let Some(err) = c.as_mut().now_or_never() {
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

    async fn receive_snapshot(
        streaming: &mut Option<Streaming<C>>,
        req: InstallSnapshotRequest<C>,
    ) -> Result<Option<Snapshot<C>>, StorageError<C::NodeId>> {
        let snapshot_meta = req.meta.clone();
        let done = req.done;

        tracing::info!(req = display(&req), "{}", func_name!());

        {
            let s = streaming.as_mut().unwrap();
            s.receive(req).await?;
        }

        tracing::info!("Done received snapshot chunk");

        if done {
            let streaming = streaming.take().unwrap();
            let mut data = streaming.snapshot_data;

            data.as_mut()
                .shutdown()
                .await
                .map_err(|e| StorageIOError::write_snapshot(Some(snapshot_meta.signature()), &e))?;

            tracing::info!("finished streaming snapshot: {:?}", snapshot_meta);
            return Ok(Some(Snapshot::new(snapshot_meta, data)));
        }

        Ok(None)
    }
}
