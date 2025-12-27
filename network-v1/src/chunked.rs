//! Chunk-based snapshot transport implementation.

use std::cmp::Ordering;
use std::future::Future;
use std::io::SeekFrom;
use std::time::Duration;

use futures::FutureExt;
use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::OptionalSend;
use openraft::Raft;
use openraft::RaftTypeConfig;
use openraft::SnapshotSegmentId;
use openraft::StorageError;
use openraft::ToStorageResult;
use openraft::error::ErrorSource;
use openraft::error::InstallSnapshotError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::ReplicationClosed;
use openraft::error::SnapshotMismatch;
use openraft::error::StreamingError;
use openraft::network::RPCOption;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::SnapshotResponse;
use openraft::storage::Snapshot;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::VoteOf;
use openraft::vote::RaftVote;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

use crate::RaftNetwork;
use crate::streaming::Streaming;

/// Compare two votes using their public API.
///
/// This replicates the logic from `RefVote::partial_cmp` using only the public
/// `RaftVote` trait methods.
fn vote_greater<C: RaftTypeConfig>(a: &C::Vote, b: &C::Vote) -> bool {
    match PartialOrd::partial_cmp(a.leader_id(), b.leader_id()) {
        Some(Ordering::Greater) => true,
        Some(Ordering::Equal) => a.is_committed() && !b.is_committed(),
        Some(Ordering::Less) => false,
        None => {
            // If two leader_ids are not comparable, use committed status
            a.is_committed() && !b.is_committed()
        }
    }
}

/// Chunk-based snapshot transport.
///
/// This implementation splits snapshots into chunks and sends/receives them incrementally.
/// It requires `SnapshotData` to implement `AsyncRead + AsyncWrite + AsyncSeek + Unpin`.
pub struct Chunked<C>(std::marker::PhantomData<C>)
where C: RaftTypeConfig;

impl<C> Chunked<C>
where
    C: RaftTypeConfig,
    C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
{
    /// Send a snapshot to a target node via `Net`.
    ///
    /// This function is for backward compatibility and provides a default implementation for
    /// `RaftNetworkV2::full_snapshot()` using `RaftNetwork::install_snapshot()`.
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

            if vote_greater::<C>(&resp.vote, &vote) {
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

    /// Receive a chunk of snapshot. If the snapshot is done receiving, return the snapshot.
    ///
    /// This method provides a default implementation for chunk-based snapshot transport
    /// and requires the caller to provide two things:
    ///
    /// - The receiving state `streaming` is maintained by the caller.
    /// - And it depends on `Raft::begin_receiving_snapshot()` to create a `SnapshotData` for
    ///   receiving data.
    pub async fn receive_snapshot(
        streaming: &mut Option<Streaming<C>>,
        raft: &Raft<C>,
        req: InstallSnapshotRequest<C>,
    ) -> Result<Option<Snapshot<C>>, RaftError<C, InstallSnapshotError>> {
        let snapshot_id = &req.meta.snapshot_id;
        let snapshot_meta = req.meta.clone();
        let done = req.done;

        tracing::info!(
            "Chunked::receive_snapshot: snapshot_id={}, offset={}, done={}",
            snapshot_id,
            req.offset,
            done
        );

        let curr_id = streaming.as_ref().map(|s| s.snapshot_id());

        if curr_id != Some(snapshot_id) {
            if req.offset != 0 {
                let mismatch = InstallSnapshotError::SnapshotMismatch(SnapshotMismatch {
                    expect: SnapshotSegmentId {
                        id: snapshot_id.clone(),
                        offset: 0,
                    },
                    got: SnapshotSegmentId {
                        id: snapshot_id.clone(),
                        offset: req.offset,
                    },
                });
                return Err(RaftError::APIError(mismatch));
            }

            // Changed to another stream. re-init snapshot state.
            let snapshot_data = raft.begin_receiving_snapshot().await.map_err(|e| {
                // Safe unwrap: `RaftError<Infallible>` is always a Fatal.
                RaftError::Fatal(e.unwrap_fatal())
            })?;

            *streaming = Some(Streaming::new(snapshot_id.clone(), snapshot_data));
        }

        {
            let s = streaming.as_mut().unwrap();
            Self::receive_chunk(s, &req).await?;
        }

        tracing::info!("Done received snapshot chunk");

        if done {
            let streaming = streaming.take().unwrap();
            let mut data = streaming.into_snapshot_data();

            data.shutdown().await.map_err(|e| {
                RaftError::Fatal(openraft::error::Fatal::from(StorageError::write_snapshot(
                    Some(snapshot_meta.signature()),
                    C::ErrorSource::from_error(&e),
                )))
            })?;

            tracing::info!("finished streaming snapshot: {:?}", snapshot_meta);
            return Ok(Some(Snapshot {
                meta: snapshot_meta,
                snapshot: data,
            }));
        }

        Ok(None)
    }

    /// Receive a single chunk of snapshot data.
    async fn receive_chunk(
        streaming: &mut Streaming<C>,
        req: &InstallSnapshotRequest<C>,
    ) -> Result<bool, StorageError<C>> {
        // Always seek to the target offset if not an exact match.
        if req.offset != streaming.offset {
            if let Err(err) = streaming.snapshot_data_mut().seek(SeekFrom::Start(req.offset)).await {
                return Err(StorageError::from_io_error(
                    ErrorSubject::Snapshot(Some(req.meta.signature())),
                    ErrorVerb::Seek,
                    err,
                ));
            }
            streaming.offset = req.offset;
        }

        // Write the next segment & update offset.
        let res = streaming.snapshot_data_mut().write_all(&req.data).await;
        if let Err(err) = res {
            return Err(StorageError::from_io_error(
                ErrorSubject::Snapshot(Some(req.meta.signature())),
                ErrorVerb::Write,
                err,
            ));
        }
        streaming.offset += req.data.len() as u64;
        Ok(req.done)
    }
}
