//! Streaming state for receiving snapshot chunks.

use std::io::SeekFrom;
use std::marker::PhantomData;

use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::SnapshotId;
use openraft::StorageError;
use openraft::type_config::alias::SnapshotSignatureOf;
use openraft_macros::since;
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

use crate::network_v1::InstallSnapshotRequest;

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
    /// Complete writes and return a handle positioned at the start of the snapshot.
    pub(crate) async fn finish(mut self, signature: SnapshotSignatureOf<C>) -> Result<SD, StorageError<C>> {
        let snapshot_error =
            |verb, error| StorageError::from_io_error(ErrorSubject::Snapshot(Some(signature.clone())), verb, error);
        self.snapshot_data.shutdown().await.map_err(|error| snapshot_error(ErrorVerb::Write, error))?;
        self.snapshot_data.rewind().await.map_err(|error| snapshot_error(ErrorVerb::Seek, error))?;
        let data = self.into_snapshot_data();
        Ok(data)
    }

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

#[cfg(test)]
mod tests {
    use openraft::type_config::TypeConfigExt;
    use openraft::type_config::alias::SnapshotMetaOf;
    use openraft::type_config::alias::VoteOf;
    use tokio::io::AsyncReadExt;

    use super::*;
    use crate::network_v1::SnapshotMeta;

    openraft::declare_raft_types!(TestConfig);

    #[test]
    fn received_file_reads_from_the_beginning() {
        TestConfig::run(async {
            let file = tempfile::tempfile().unwrap();
            let file = tokio::fs::File::from_std(file);
            let meta = SnapshotMeta::new(SnapshotMetaOf::<TestConfig>::default(), "transfer".to_string());
            let request = InstallSnapshotRequest::<TestConfig> {
                vote: VoteOf::<TestConfig>::new(1, 1),
                meta,
                offset: 0,
                data: b"complete snapshot".to_vec(),
                done: true,
            };
            let mut streaming = Streaming::<TestConfig, _>::new("transfer".to_string(), file);
            streaming.receive_chunk(&request).await.unwrap();
            let mut installed = streaming.finish(request.meta.signature()).await.unwrap();
            let mut actual = Vec::new();
            installed.read_to_end(&mut actual).await.unwrap();
            assert_eq!(actual, request.data);
        });
    }
}
