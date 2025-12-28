//! Streaming state for receiving snapshot chunks.

use std::io::SeekFrom;
use std::sync::Arc;

use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::RaftTypeConfig;
use openraft::SnapshotId;
use openraft::StorageError;
use openraft::raft::InstallSnapshotRequest;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::MutexOf;
use openraft_macros::since;
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

/// The Raft node is streaming in a snapshot from the leader.
#[since(version = "0.10.0")]
pub struct Streaming<C>
where C: RaftTypeConfig
{
    /// The offset of the last byte written to the snapshot.
    offset: u64,

    /// The ID of the snapshot being written.
    snapshot_id: SnapshotId,

    /// A handle to the snapshot writer.
    snapshot_data: C::SnapshotData,
}

impl<C> Streaming<C>
where C: RaftTypeConfig
{
    #[since(version = "0.10.0")]
    pub fn new(snapshot_id: SnapshotId, snapshot_data: C::SnapshotData) -> Self {
        Self {
            offset: 0,
            snapshot_id,
            snapshot_data,
        }
    }

    /// Get the snapshot ID for this streaming snapshot.
    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    /// Consumes the `Streaming` and returns the snapshot data.
    pub fn into_snapshot_data(self) -> C::SnapshotData {
        self.snapshot_data
    }
}

impl<C> Streaming<C>
where
    C: RaftTypeConfig,
    C::SnapshotData: tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
{
    /// Receive a single chunk of snapshot data.
    ///
    /// Writes the chunk data to the snapshot at the specified offset.
    /// Returns `true` if this was the final chunk.
    pub async fn receive_chunk(&mut self, req: &InstallSnapshotRequest<C>) -> Result<bool, StorageError<C>> {
        // Seek to the target offset if not an exact match.
        if req.offset != self.offset {
            if let Err(err) = self.snapshot_data.seek(SeekFrom::Start(req.offset)).await {
                return Err(StorageError::from_io_error(
                    ErrorSubject::Snapshot(Some(req.meta.signature())),
                    ErrorVerb::Seek,
                    err,
                ));
            }
            self.offset = req.offset;
        }

        // Write the chunk data.
        if let Err(err) = self.snapshot_data.write_all(&req.data).await {
            return Err(StorageError::from_io_error(
                ErrorSubject::Snapshot(Some(req.meta.signature())),
                ErrorVerb::Write,
                err,
            ));
        }
        self.offset += req.data.len() as u64;

        Ok(req.done)
    }
}

/// Shared state for receiving snapshot chunks, stored in [`Extensions`].
///
/// This wrapper holds the ongoing snapshot reception state and is stored
/// in [`Raft::extensions()`] to track chunk-based snapshot transfers.
///
/// [`Extensions`]: openraft::Extensions
/// [`Raft::extensions()`]: openraft::Raft::extensions
#[derive(Clone)]
pub struct StreamingState<C: RaftTypeConfig> {
    pub(crate) streaming: Arc<MutexOf<C, Option<Streaming<C>>>>,
}

impl<C: RaftTypeConfig> StreamingState<C> {
    /// Create a new empty streaming state.
    pub fn new() -> Self {
        Self {
            streaming: Arc::new(C::mutex(None)),
        }
    }
}

impl<C: RaftTypeConfig> Default for StreamingState<C> {
    fn default() -> Self {
        Self::new()
    }
}
