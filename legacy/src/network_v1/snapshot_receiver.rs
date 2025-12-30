//! Extension trait for `Raft` to support chunk-based snapshot receiving.
//!
//! This module provides [`ChunkedSnapshotReceiver`], an extension trait that adds
//! chunk-based snapshot receiving via `install_snapshot()` to [`openraft::Raft`].

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

use super::streaming::Streaming;
use super::streaming::StreamingState;

/// Extension trait for `Raft` to support chunk-based snapshot receiving.
///
/// This trait adds the `install_snapshot()` method for receiving snapshot chunks
/// via the v1 network protocol.
///
/// The streaming state is stored via `Raft::extension()` as `StreamingState<C>`.
///
/// # Usage
///
/// ```ignore
/// use openraft_legacy::network_v1::ChunkedSnapshotReceiver;
///
/// let raft = openraft::Raft::new(...).await?;
///
/// // Standard Raft methods
/// raft.client_write(...).await?;
///
/// // Added method for chunked snapshot receiving (via trait)
/// raft.install_snapshot(req).await?;
/// ```
pub trait ChunkedSnapshotReceiver<C: RaftTypeConfig>: private::Sealed<C>
where C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin
{
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
    fn install_snapshot(
        &self,
        req: InstallSnapshotRequest<C>,
    ) -> impl std::future::Future<Output = Result<InstallSnapshotResponse<C>, RaftError<C, InstallSnapshotError>>>;
}

impl<C: RaftTypeConfig> ChunkedSnapshotReceiver<C> for Raft<C>
where C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin
{
    async fn install_snapshot(
        &self,
        req: InstallSnapshotRequest<C>,
    ) -> Result<InstallSnapshotResponse<C>, RaftError<C, InstallSnapshotError>> {
        let vote = req.vote.clone();
        let snapshot_id = &req.meta.snapshot_id;
        let snapshot_meta = req.meta.clone();
        let done = req.done;

        tracing::info!(
            snapshot_id = display(snapshot_id),
            offset = req.offset,
            done,
            "ChunkedSnapshotReceiver::install_snapshot"
        );

        // Get or create streaming state via extension()
        let state: StreamingState<C> = self.extension();
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
                self.begin_receiving_snapshot().await.map_err(|e| RaftError::Fatal(e.unwrap_fatal()))?;

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

            self.install_full_snapshot(vote.clone(), snapshot).await.map_err(RaftError::Fatal)?;
        }

        // Return response with current vote from metrics
        let my_vote = self.metrics().borrow_watched().vote.clone();

        Ok(InstallSnapshotResponse { vote: my_vote })
    }
}

mod private {
    use openraft::Raft;
    use openraft::RaftTypeConfig;

    pub trait Sealed<C: RaftTypeConfig> {}

    impl<C: RaftTypeConfig> Sealed<C> for Raft<C> {}
}
