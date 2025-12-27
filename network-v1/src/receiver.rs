//! Server-side chunk-based snapshot receiving.

use std::io::SeekFrom;

use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::Raft;
use openraft::RaftTypeConfig;
use openraft::SnapshotSegmentId;
use openraft::StorageError;
use openraft::error::ErrorSource;
use openraft::error::InstallSnapshotError;
use openraft::error::RaftError;
use openraft::error::SnapshotMismatch;
use openraft::raft::InstallSnapshotRequest;
use openraft::storage::Snapshot;
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

use crate::streaming::Streaming;

/// Receives snapshot chunks and assembles them into a complete snapshot.
///
/// This is the server-side (Follower) component for chunk-based snapshot transport.
/// It receives chunks from a leader and assembles them into a complete snapshot.
pub struct Receiver<C>(std::marker::PhantomData<C>)
where C: RaftTypeConfig;

impl<C> Receiver<C>
where
    C: RaftTypeConfig,
    C::SnapshotData: tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
{
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
            "Receiver::receive_snapshot: snapshot_id={}, offset={}, done={}",
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
