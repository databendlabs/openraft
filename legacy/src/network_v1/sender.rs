//! Client-side chunk-based snapshot sending.

use std::future::Future;
use std::io::SeekFrom;
use std::time::Duration;

use futures::FutureExt;
use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::ToStorageResult;
use openraft::error::InstallSnapshotError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::network::RPCOption;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::SnapshotResponse;
use openraft::storage::Snapshot;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::VoteOf;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;

use super::RaftNetwork;

/// Sends snapshots in chunks via `RaftNetwork::install_snapshot()`.
///
/// This is the client-side (Leader) component for chunk-based snapshot transport.
/// It splits a snapshot into chunks and sends them incrementally to a follower.
pub struct Sender<C>(std::marker::PhantomData<C>)
where C: RaftTypeConfig;

impl<C> Sender<C>
where
    C: RaftTypeConfig,
    C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin,
{
    /// Send a snapshot to a target node via `Net`.
    ///
    /// This function provides a default implementation for `RaftNetworkV2::full_snapshot()`
    /// using `RaftNetwork::install_snapshot()`.
    ///
    /// The argument `vote` is the leader's (the caller's) vote,
    /// which is used to check if the leader is still valid by a follower.
    ///
    /// `cancel` is a future that is polled by this function to check if the caller decides to
    /// cancel. It returns `Ready` if the caller decides to cancel this snapshot transmission.
    pub async fn send_snapshot<Net>(
        net: &mut Net,
        vote: VoteOf<C>,
        mut snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>>
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
            // yields. Because network implementation does not yield.
            C::sleep(Duration::from_millis(1)).await;

            snapshot.snapshot.seek(SeekFrom::Start(offset)).await.sto_res(subject_verb)?;

            // Safe unwrap(): this function is called only by default implementation of
            // `RaftNetwork::full_snapshot()` and it is always set.
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
                vote: vote.clone(),
                meta: snapshot.meta.clone(),
                offset,
                data: buf,
                done,
            };

            // Send the RPC over to the target.
            tracing::debug!(
                "sending snapshot chunk: snapshot_size: {}, offset: {}, end: {}, done: {}",
                req.data.len(),
                req.offset,
                end,
                req.done
            );

            #[allow(deprecated)]
            let res = C::timeout(option.hard_ttl(), net.install_snapshot(req, option.clone())).await;

            let resp = match res {
                Ok(outer_res) => match outer_res {
                    Ok(res) => res,
                    Err(err) => {
                        let err: RPCError<C, RaftError<C, InstallSnapshotError>> = err;

                        tracing::warn!("failed to send InstallSnapshot RPC: {}", err);

                        match err {
                            RPCError::Timeout(_) => {}
                            RPCError::Unreachable(_) => {}
                            RPCError::Network(_) => {}
                            RPCError::RemoteError(remote_err) => match remote_err.source {
                                RaftError::Fatal(_) => {}
                                RaftError::APIError(snapshot_err) => match snapshot_err {
                                    InstallSnapshotError::SnapshotMismatch(mismatch) => {
                                        tracing::warn!(
                                            "snapshot mismatch, reset offset and retry: mismatch: {}",
                                            mismatch
                                        );
                                        offset = 0;
                                    }
                                },
                            },
                        }
                        continue;
                    }
                },
                Err(err) => {
                    tracing::warn!("timeout sending InstallSnapshot RPC: {}", err);
                    continue;
                }
            };

            if resp.vote != vote {
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
}
