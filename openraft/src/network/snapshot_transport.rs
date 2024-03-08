//! Provide a default chunked snapshot transport implementation for SnapshotData that implements
//! AsyncWrite + AsyncRead + AsyncSeek + Unpin.

use std::future::Future;
use std::io::SeekFrom;
use std::time::Duration;

use futures::FutureExt;
use macros::add_async_trait;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

use crate::error::Fatal;
use crate::error::RaftError;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::network::RPCOption;
use crate::raft::InstallSnapshotRequest;
use crate::raft::SnapshotResponse;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::AsyncRuntime;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::OptionalSend;
use crate::Raft;
use crate::RaftNetwork;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::SnapshotId;
use crate::StorageError;
use crate::StorageIOError;
use crate::ToStorageResult;
use crate::Vote;

/// Defines the sending and receiving API for snapshot transport.
#[add_async_trait]
pub trait SnapshotTransport<C: RaftTypeConfig> {
    /// Send a snapshot to a target node via `Net`.
    ///
    /// This function is for backward compatibility and provides a default implement for
    /// `RaftNetwork::full_snapshot()` upon `RafNetwork::install_snapshot()`.
    ///
    /// The argument `vote` is the leader's(the caller's) vote,
    /// which is used to check if the leader is still valid by a follower.
    ///
    /// `cancel` is a future that is polled by this function to check if the caller decides to
    /// cancel.
    /// It return `Ready` if the caller decide to cancel this snapshot transmission.
    // TODO: consider removing dependency on RaftNetwork
    async fn send_snapshot<Net>(
        net: &mut Net,
        vote: Vote<C::NodeId>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>>
    where
        Net: RaftNetwork<C> + ?Sized;

    /// Receive a chunk of snapshot. If the snapshot is done receiving, return the snapshot.
    ///
    /// This method provide a default implementation for chunk based snapshot transport,
    /// and requires the caller to provide two things:
    ///
    /// - The receiving state `streaming` is maintained by the caller.
    /// - And it depends on `Raft::begin_receiving_snapshot()` to create a `SnapshotData` for
    /// receiving data.
    ///
    /// Example usage:
    /// ```ignore
    /// struct App<C> {
    ///     raft: Raft<C>
    ///     streaming: Option<Streaming<C>>,
    /// }
    ///
    /// impl<C> App<C> {
    ///     fn handle_install_snapshot_request(&mut self, req: InstallSnapshotRequest<C>) {
    ///         let res = Chunked::receive_snapshot(&mut self.streaming, &self.raft, req).await?;
    ///         if let Some(snapshot) = res {
    ///             self.raft.install_snapshot(snapshot).await?;
    ///         }
    ///     }
    /// }
    /// ```
    async fn receive_snapshot(
        streaming: &mut Option<Streaming<C>>,
        raft: &Raft<C>,
        req: InstallSnapshotRequest<C>,
    ) -> Result<Option<Snapshot<C>>, RaftError<C::NodeId, crate::error::InstallSnapshotError>>;
}

/// Send and Receive snapshot by chunks.
pub struct Chunked {}

/// This chunk based implementation requires `SnapshotData` to be `AsyncRead + AsyncSeek`.
impl<C: RaftTypeConfig> SnapshotTransport<C> for Chunked
where C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin
{
    async fn send_snapshot<Net>(
        net: &mut Net,
        vote: Vote<C::NodeId>,
        mut snapshot: Snapshot<C>,
        mut cancel: impl Future<Output = ReplicationClosed> + OptionalSend,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>>
    where
        Net: RaftNetwork<C> + ?Sized,
    {
        let subject_verb = || (ErrorSubject::Snapshot(Some(snapshot.meta.signature())), ErrorVerb::Read);

        let mut offset = 0;
        let end = snapshot.snapshot.seek(SeekFrom::End(0)).await.sto_res(subject_verb)?;

        let mut c = std::pin::pin!(cancel);
        loop {
            // If canceled, return at once
            if let Some(err) = c.as_mut().now_or_never() {
                return Err(err.into());
            }

            // Sleep a short time otherwise in test environment it is a dead-loop that never
            // yields.
            // Because network implementation does not yield.
            AsyncRuntimeOf::<C>::sleep(Duration::from_millis(10)).await;

            snapshot.snapshot.seek(SeekFrom::Start(offset)).await.sto_res(subject_verb)?;

            // Safe unwrap(): this function is called only by default implementation of
            // `RaftNetwork::full_snapshot()` and it is always set.
            let chunk_size = option.snapshot_chunk_size().unwrap();
            let mut buf = Vec::with_capacity(chunk_size);
            while buf.capacity() > buf.len() {
                let n = snapshot.snapshot.read_buf(&mut buf).await.sto_res(subject_verb)?;
                if n == 0 {
                    break;
                }
            }

            let n_read = buf.len();

            let done = (offset + n_read as u64) == end;
            let req = InstallSnapshotRequest {
                vote,
                meta: snapshot.meta.clone(),
                offset,
                data: buf,
                done,
            };

            // Send the RPC over to the target.
            tracing::debug!(
                snapshot_size = req.data.len(),
                req.offset,
                end,
                req.done,
                "sending snapshot chunk"
            );

            #[allow(deprecated)]
            let res = AsyncRuntimeOf::<C>::timeout(option.hard_ttl(), net.install_snapshot(req, option.clone())).await;

            let resp = match res {
                Ok(outer_res) => match outer_res {
                    Ok(res) => res,
                    Err(err) => {
                        tracing::warn!(error=%err, "error sending InstallSnapshot RPC to target");
                        continue;
                    }
                },
                Err(err) => {
                    tracing::warn!(error=%err, "timeout while sending InstallSnapshot RPC to target");
                    continue;
                }
            };

            if resp.vote > vote {
                // Unfinished, return a response with a higher vote.
                // The caller checks the vote and return a HigherVote error.
                return Ok(SnapshotResponse::new(resp.vote));
            }

            if done {
                return Ok(SnapshotResponse::new(resp.vote));
            }

            offset += n_read as u64;
        }
    }

    async fn receive_snapshot(
        streaming: &mut Option<Streaming<C>>,
        raft: &Raft<C>,
        req: InstallSnapshotRequest<C>,
    ) -> Result<Option<Snapshot<C>>, RaftError<C::NodeId, crate::error::InstallSnapshotError>> {
        let snapshot_id = &req.meta.snapshot_id;
        let snapshot_meta = req.meta.clone();
        let done = req.done;

        tracing::info!(req = display(&req), "{}", func_name!());

        let curr_id = streaming.as_ref().map(|s| s.snapshot_id());

        if curr_id != Some(snapshot_id) {
            if req.offset != 0 {
                let mismatch = crate::error::InstallSnapshotError::SnapshotMismatch(crate::error::SnapshotMismatch {
                    expect: crate::SnapshotSegmentId {
                        id: snapshot_id.clone(),
                        offset: 0,
                    },
                    got: crate::SnapshotSegmentId {
                        id: snapshot_id.clone(),
                        offset: req.offset,
                    },
                });
                return Err(RaftError::APIError(mismatch));
            }

            // Changed to another stream. re-init snapshot state.
            let snapshot_data = raft.begin_receiving_snapshot().await.map_err(|e| {
                // Safe unwrap: `RaftError<Infallible>` is always a Fatal.
                RaftError::Fatal(e.into_fatal().unwrap())
            })?;

            *streaming = Some(Streaming::new(snapshot_id.clone(), snapshot_data));
        }

        {
            let s = streaming.as_mut().unwrap();
            s.receive(req).await?;
        }

        tracing::info!("Done received snapshot chunk");

        if done {
            let streaming = streaming.take().unwrap();
            let mut data = streaming.into_snapshot_data();

            data.as_mut().shutdown().await.map_err(|e| {
                let io_err = StorageIOError::write_snapshot(Some(snapshot_meta.signature()), &e);
                StorageError::from(io_err)
            })?;

            tracing::info!("finished streaming snapshot: {:?}", snapshot_meta);
            return Ok(Some(Snapshot::new(snapshot_meta, data)));
        }

        Ok(None)
    }
}

/// The Raft node is streaming in a snapshot from the leader.
pub struct Streaming<C>
where C: RaftTypeConfig
{
    /// The offset of the last byte written to the snapshot.
    offset: u64,

    /// The ID of the snapshot being written.
    snapshot_id: SnapshotId,

    /// A handle to the snapshot writer.
    snapshot_data: Box<C::SnapshotData>,
}

impl<C> Streaming<C>
where C: RaftTypeConfig
{
    pub fn new(snapshot_id: SnapshotId, snapshot_data: Box<C::SnapshotData>) -> Self {
        Self {
            offset: 0,
            snapshot_id,
            snapshot_data,
        }
    }

    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    /// Consumes the `Streaming` and returns the snapshot data.
    pub fn into_snapshot_data(self) -> Box<C::SnapshotData> {
        self.snapshot_data
    }
}

impl<C> Streaming<C>
where
    C: RaftTypeConfig,
    C::SnapshotData: tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
{
    /// Receive a chunk of snapshot data.
    pub async fn receive(&mut self, req: InstallSnapshotRequest<C>) -> Result<bool, StorageError<C::NodeId>> {
        // TODO: check id?

        // Always seek to the target offset if not an exact match.
        if req.offset != self.offset {
            if let Err(err) = self.snapshot_data.as_mut().seek(SeekFrom::Start(req.offset)).await {
                return Err(StorageError::from_io_error(
                    ErrorSubject::Snapshot(Some(req.meta.signature())),
                    ErrorVerb::Seek,
                    err,
                ));
            }
            self.offset = req.offset;
        }

        // Write the next segment & update offset.
        let res = self.snapshot_data.as_mut().write_all(&req.data).await;
        if let Err(err) = res {
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
