use macros::add_async_trait;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;

use crate::core::snapshot_state::SnapshotRequestId;
use crate::core::streaming_state::Streaming;
use crate::raft::InstallSnapshotRequest;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::StorageError;
use crate::StorageIOError;

#[add_async_trait]
pub(crate) trait SnapshotTransport<C: RaftTypeConfig> {
    async fn receive_snapshot(
        _streaming: &mut Option<Streaming<C>>,
        _req: InstallSnapshotRequest<C>,
    ) -> Result<Option<Snapshot<C>>, StorageError<C::NodeId>> {
        unimplemented!("receive_snapshot is only implemented with SnapshotData with AsyncRead + AsyncSeek")
    }
}

/// receive snapshot by chunks.
pub(crate) struct Chunked {}

impl<C: RaftTypeConfig> SnapshotTransport<C> for Chunked
where C::SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + OptionalSend + OptionalSync + Unpin + 'static
{
    async fn receive_snapshot(
        streaming: &mut Option<Streaming<C>>,
        req: InstallSnapshotRequest<C>,
    ) -> Result<Option<Snapshot<C>>, StorageError<C::NodeId>> {
        let snapshot_meta = req.meta.clone();
        let done = req.done;
        let offset = req.offset;

        let req_id = SnapshotRequestId::new(*req.vote.leader_id(), snapshot_meta.snapshot_id.clone(), offset);

        tracing::info!(
            req = display(&req),
            snapshot_req_id = debug(&req_id),
            "{}",
            func_name!()
        );

        {
            let s = streaming.as_mut().unwrap();
            s.receive(req).await?;
        }

        tracing::info!(snapshot_req_id = debug(&req_id), "received snapshot chunk");

        if done {
            let streaming = streaming.take().unwrap();
            let mut data = streaming.snapshot_data;

            data.as_mut()
                .shutdown()
                .await
                .map_err(|e| StorageIOError::write_snapshot(Some(snapshot_meta.signature()), &e))?;

            tracing::info!("finished streaming snapshot: {:?}", snapshot_meta);
            return Ok(Some(Snapshot::new(snapshot_meta, data)));
        }

        Ok(None)
    }
}
