//! Raft wrapper with `install_snapshot()` for receiving snapshot chunks.
//!
//! This module provides [`ChunkedRaft`], a wrapper around [`openraft::Raft`] that adds
//! chunk-based snapshot receiving via `install_snapshot()`. It derefs to the inner
//! Raft, so all standard Raft methods are available.

use openraft::Raft;
use openraft::RaftTypeConfig;
use openraft::async_runtime::Mutex;
use openraft::async_runtime::WatchReceiver;
use openraft::error::InstallSnapshotError;
use openraft::error::RaftError;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;

use crate::receiver::Receiver;
use crate::streaming::StreamingState;

/// Raft wrapper with `install_snapshot()` for chunk-based snapshot receiving.
///
/// This wrapper adds the `install_snapshot()` method for receiving snapshot chunks
/// via the v1 network protocol. It derefs to [`openraft::Raft`], so all standard
/// Raft methods are directly accessible.
///
/// The streaming state is stored in `Raft::extensions()` as `StreamingState<C>`.
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
    /// 1. Getting or creating the streaming state from `Raft::extensions()`
    /// 2. Receiving the chunk via `Receiver::receive_snapshot()`
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

        // Get or create streaming state from extensions
        let state: StreamingState<C> = self.inner.extensions().get_or_default();
        let mut streaming = state.streaming.lock().await;

        // Receive chunk and check if snapshot is complete
        let snapshot = Receiver::<C>::receive_snapshot(&mut *streaming, &self.inner, req).await?;

        if let Some(snapshot) = snapshot {
            // All chunks received, install the full snapshot
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
