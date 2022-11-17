use std::marker::PhantomData;

use anyerror::AnyError;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;

use crate::raft::InstallSnapshotRequest;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::RaftTypeConfig;
use crate::SnapshotId;
use crate::StorageError;
use crate::StorageIOError;

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
where SD: AsyncWrite + Unpin
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

        if req.offset != self.offset {
            let sto_io_err = StorageIOError::new(
                ErrorSubject::Snapshot(req.meta.signature()),
                ErrorVerb::Write,
                AnyError::error(format!("offsets do not match {}:{}", self.offset, req.offset)),
            );
            return Err(StorageError::IO { source: sto_io_err });
        }

        // Write the next segment & update offset.
        let res = self.snapshot_data.as_mut().write_all(&req.data).await;
        if let Err(err) = res {
            return Err(StorageError::from_io_error(
                ErrorSubject::Snapshot(req.meta.signature()),
                ErrorVerb::Write,
                err,
            ));
        }
        self.offset += req.data.len() as u64;
        Ok(req.done)
    }
}
