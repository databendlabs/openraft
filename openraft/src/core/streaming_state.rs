use std::marker::PhantomData;

use futures::Sink;
use futures::SinkExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;

use crate::raft::InstallSnapshotRequest;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::RaftTypeConfig;
use crate::SnapshotId;
use crate::StorageError;

/// The Raft node is streaming in a snapshot from the leader.
pub(crate) struct StreamingState<C: RaftTypeConfig, SD> {
    /// The offset of the last byte written to the snapshot.
    pub(crate) offset: u64,
    /// The ID of the snapshot being written.
    pub(crate) snapshot_id: SnapshotId,
    /// A handle to the snapshot writer.
    pub(crate) snapshot_data: Box<SD>,

    _p: PhantomData<C>,
}

impl<C: RaftTypeConfig, SD> StreamingState<C, SD>
where SD: Sink<C::SD, Error = std::io::Error> + Unpin
{
    pub(crate) fn new(snapshot_id: SnapshotId, snapshot_data: Box<SD>) -> Self {
        Self {
            offset: 0,
            snapshot_id,
            snapshot_data,
            _p: Default::default(),
        }
    }

    /// Receive a chunk of snapshot data.
    pub(crate) async fn receive(&mut self, req: InstallSnapshotRequest<C>) -> Result<bool, StorageError<C::NodeId>> {
        // TODO: check id?

        // Always seek to the target offset if not an exact match.
        if req.offset != self.offset {
            return Err(StorageError::from_io_error(
                ErrorSubject::Snapshot(req.meta.signature()),
                ErrorVerb::Write,
                std::io::ErrorKind::Other.into(),
            ));
        }

        let done = req.data.is_none();

        // Write the next segment & update offset.
        if let Some(data) = req.data {
            let res = self.snapshot_data.feed(data).await;
            if let Err(err) = res {
                return Err(StorageError::from_io_error(
                    ErrorSubject::Snapshot(req.meta.signature()),
                    ErrorVerb::Write,
                    err,
                ));
            }
        }

        self.offset += 1;
        Ok(done)
    }
}
