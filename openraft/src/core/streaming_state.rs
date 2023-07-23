use crate::raft::SnapshotChunk;
use crate::raft::SnapshotData;
use crate::raft::SnapshotManifest;
use crate::type_config::RTCSnapshotChunk;
use crate::type_config::RTCSnapshotData;
use crate::type_config::RTCSnapshotManifest;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::StorageError;
use crate::ToStorageResult;

/// The Raft node is streaming in a snapshot from the leader.
pub(crate) struct Streaming<C>
where C: RaftTypeConfig
{
    /// The ID of the snapshot being written.
    pub(crate) snapshot_meta: SnapshotMeta<C::NodeId, C::Node>,

    /// A handle to the snapshot writer.
    pub(crate) streaming_data: Box<RTCSnapshotData<C>>,

    pub(crate) manifest: RTCSnapshotManifest<C>,
}

impl<C> Streaming<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(
        snapshot_meta: SnapshotMeta<C::NodeId, C::Node>,
        manifest: RTCSnapshotManifest<C>,
        streaming_data: Box<RTCSnapshotData<C>>,
    ) -> Self {
        Self {
            snapshot_meta,
            manifest,
            streaming_data,
        }
    }

    /// Receive a chunk of snapshot data. Returns true if it was a new chunk
    pub(crate) async fn receive(&mut self, chunk: RTCSnapshotChunk<C>) -> Result<bool, StorageError<C::NodeId>> {
        let chunk_id = chunk.id();
        let err_x = || {
            (
                ErrorSubject::Snapshot(Some(self.snapshot_meta.signature())),
                ErrorVerb::Write,
            )
        };

        self.streaming_data.as_mut().receive(chunk).await.sto_res(err_x)?;

        self.manifest.receive(&chunk_id).sto_res(err_x)
    }
}
