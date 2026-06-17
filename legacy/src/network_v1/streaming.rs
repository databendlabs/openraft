//! Streaming state for receiving snapshot chunks.

use std::io::SeekFrom;
use std::marker::PhantomData;
use std::sync::Arc;

use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::SnapshotId;
use openraft::StorageError;
use openraft::raft::InstallSnapshotRequest;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::MutexOf;
use openraft_macros::since;
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

/// State for receiving a chunked snapshot from the leader.
///
/// This struct accumulates snapshot chunks as they arrive and assembles them
/// into a complete snapshot. Once all chunks are received, the snapshot data
/// can be extracted and installed into the Raft state machine.
#[since(version = "0.10.0")]
pub struct Streaming<C, SD>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    /// The offset of the last byte written to the snapshot.
    offset: u64,

    /// The ID of the snapshot being written.
    snapshot_id: SnapshotId,

    /// A handle to the snapshot writer.
    snapshot_data: SD,

    _phantom: PhantomData<fn() -> C>,
}

impl<C, SD> Streaming<C, SD>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    #[since(version = "0.10.0")]
    pub fn new(snapshot_id: SnapshotId, snapshot_data: SD) -> Self {
        Self {
            offset: 0,
            snapshot_id,
            snapshot_data,
            _phantom: PhantomData,
        }
    }

    /// Get the snapshot ID for this streaming snapshot.
    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    /// Consumes the `Streaming` and returns the snapshot data.
    pub fn into_snapshot_data(self) -> SD {
        self.snapshot_data
    }
}

impl<C, SD> Streaming<C, SD>
where
    C: RaftTypeConfig,
    SD: tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin + OptionalSend + 'static,
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

/// Shared state for receiving snapshot chunks, stored via [`Raft::extension()`].
///
/// This wrapper holds the ongoing snapshot reception state and is stored
/// via [`Raft::extension()`] to track chunk-based snapshot transfers.
///
/// [`Raft::extension()`]: openraft::Raft::extension
pub struct StreamingState<C: RaftTypeConfig, SD: OptionalSend + 'static> {
    pub(crate) streaming: Arc<MutexOf<C, Option<Streaming<C, SD>>>>,
}

impl<C: RaftTypeConfig, SD: OptionalSend + 'static> Clone for StreamingState<C, SD> {
    fn clone(&self) -> Self {
        Self {
            streaming: self.streaming.clone(),
        }
    }
}

impl<C: RaftTypeConfig, SD: OptionalSend + 'static> StreamingState<C, SD> {
    /// Create a new empty streaming state.
    pub fn new() -> Self {
        Self {
            streaming: Arc::new(C::mutex(None)),
        }
    }
}

impl<C: RaftTypeConfig, SD: OptionalSend + 'static> Default for StreamingState<C, SD> {
    fn default() -> Self {
        Self::new()
    }
}
