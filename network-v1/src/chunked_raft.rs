//! Raft wrapper with `install_snapshot()` for receiving snapshot chunks.
//!
//! This module provides [`ChunkedRaft`], a wrapper around [`openraft::Raft`] that adds
//! chunk-based snapshot receiving via `install_snapshot()`. It derefs to the inner
//! Raft, so all standard Raft methods are available.

use openraft::Raft;
use openraft::RaftTypeConfig;
use openraft::SnapshotSegmentId;
use openraft::StorageError;
use openraft::async_runtime::Mutex;
use openraft::async_runtime::WatchReceiver;
use openraft::error::ErrorSource;
use openraft::error::InstallSnapshotError;
use openraft::error::RaftError;
use openraft::error::SnapshotMismatch;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::storage::Snapshot;
use tokio::io::AsyncWriteExt;

use crate::streaming::Streaming;
use crate::streaming::StreamingState;

/// Raft wrapper with `install_snapshot()` for chunk-based snapshot receiving.
///
/// This wrapper adds the `install_snapshot()` method for receiving snapshot chunks
/// via the v1 network protocol. It derefs to [`openraft::Raft`], so all standard
/// Raft methods are directly accessible.
///
/// The streaming state is stored via `Raft::extension()` as `StreamingState<C>`.
///
/// # Usage
///
/// ```ignore
/// use openraft_network_v1::ChunkedRaft;
///
/// let inner = openraft::Raft::new(...).await?;
/// let raft = ChunkedRaft::new(inner);
///
/// // Standard Raft methods via Deref
/// raft.client_write(...).await?;
///
/// // Added method for chunked snapshot receiving
/// raft.install_snapshot(req).await?;
/// ```
pub struct ChunkedRaft<C: RaftTypeConfig> {
    inner: Raft<C>,
}

impl<C: RaftTypeConfig> ChunkedRaft<C> {
    /// Create a new `ChunkedRaft` wrapper around an [`openraft::Raft`] instance.
    pub fn new(inner: Raft<C>) -> Self {
        Self { inner }
    }

    /// Receive a snapshot chunk and assemble it into a complete snapshot.
    ///
    /// This method should be called from your RPC handler when receiving an
    /// `InstallSnapshotRequest`. It handles:
    ///
    /// 1. Getting or creating the streaming state via `Raft::extension()`
    /// 2. Receiving chunks via `Streaming::receive_chunk()`
    /// 3. When all chunks are received, calling `Raft::install_full_snapshot()`
    ///
    /// # Returns
    ///
    /// - `Ok(response)` with the current vote on success
    /// - `Err(RaftError::APIError(InstallSnapshotError::SnapshotMismatch(...)))` if chunks arrive
    ///   out of order
    /// - `Err(RaftError::Fatal(...))` on fatal errors
    pub async fn install_snapshot(
        &self,
        req: InstallSnapshotRequest<C>,
    ) -> Result<InstallSnapshotResponse<C>, RaftError<C, InstallSnapshotError>>
    where
        C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
    {
        let vote = req.vote.clone();
        let snapshot_id = &req.meta.snapshot_id;
        let snapshot_meta = req.meta.clone();
        let done = req.done;

        tracing::info!(
            snapshot_id = display(snapshot_id),
            offset = req.offset,
            done,
            "ChunkedRaft::install_snapshot"
        );

        // Get or create streaming state via extension()
        let state: StreamingState<C> = self.inner.extension();
        let mut streaming = state.streaming.lock().await;

        // Check if this is a new snapshot or continuation
        let curr_id = streaming.as_ref().map(|s| s.snapshot_id());

        if curr_id != Some(snapshot_id) {
            // New snapshot - must start at offset 0
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

            // Initialize new streaming state
            let snapshot_data =
                self.inner.begin_receiving_snapshot().await.map_err(|e| RaftError::Fatal(e.unwrap_fatal()))?;

            *streaming = Some(Streaming::new(snapshot_id.clone(), snapshot_data));
        }

        // Write the chunk
        streaming.as_mut().unwrap().receive_chunk(&req).await?;

        tracing::info!("Received snapshot chunk");

        // If done, finalize the snapshot
        if done {
            let streaming = streaming.take().unwrap();
            let mut data = streaming.into_snapshot_data();

            data.shutdown().await.map_err(|e| {
                RaftError::Fatal(openraft::error::Fatal::from(StorageError::write_snapshot(
                    Some(snapshot_meta.signature()),
                    C::ErrorSource::from_error(&e),
                )))
            })?;

            tracing::info!(snapshot_meta = debug(&snapshot_meta), "Finished streaming snapshot");

            let snapshot = Snapshot {
                meta: snapshot_meta,
                snapshot: data,
            };

            self.inner.install_full_snapshot(vote.clone(), snapshot).await.map_err(RaftError::Fatal)?;
        }

        // Return response with current vote from metrics
        let my_vote = self.inner.metrics().borrow_watched().vote.clone();

        Ok(InstallSnapshotResponse { vote: my_vote })
    }
}

impl<C: RaftTypeConfig> Clone for ChunkedRaft<C> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<C: RaftTypeConfig> std::ops::Deref for ChunkedRaft<C> {
    type Target = Raft<C>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
