//! Provide a default chunked snapshot transport implementation for SnapshotData that implements
//! AsyncWrite + AsyncRead + AsyncSeek + Unpin.

mod tokio_rt {
    #![cfg(feature = "tokio-rt")]
    //! This module contains the code that is only needed under the `tokio-rt`
    //! feature.

    use std::future::Future;
    use std::io::SeekFrom;
    use std::time::Duration;

    use futures::FutureExt;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncSeekExt;
    use tokio::io::AsyncWriteExt;

    use super::Chunked;
    use super::SnapshotTransport;
    use super::Streaming;
    use crate::ErrorSubject;
    use crate::ErrorVerb;
    use crate::OptionalSend;
    use crate::Raft;
    use crate::RaftNetwork;
    use crate::RaftTypeConfig;
    use crate::StorageError;
    use crate::ToStorageResult;
    use crate::error::InstallSnapshotError;
    use crate::error::RPCError;
    use crate::error::RaftError;
    use crate::error::ReplicationClosed;
    use crate::error::StorageIOResult;
    use crate::error::StreamingError;
    use crate::network::RPCOption;
    use crate::raft::InstallSnapshotRequest;
    use crate::raft::SnapshotResponse;
    use crate::storage::Snapshot;
    use crate::type_config::TypeConfigExt;
    use crate::type_config::alias::VoteOf;
    use crate::vote::raft_vote::RaftVoteExt;

    /// This chunk-based implementation requires `SnapshotData` to be `AsyncRead + AsyncSeek`.
    impl<C: RaftTypeConfig> SnapshotTransport<C> for Chunked
    where C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin
    {
        async fn send_snapshot<Net>(
            net: &mut Net,
            vote: VoteOf<C>,
            mut snapshot: Snapshot<C>,
            cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
            option: RPCOption,
        ) -> Result<SnapshotResponse<C>, StreamingError<C>>
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
                C::sleep(Duration::from_millis(1)).await;

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
                    vote: vote.clone(),
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
                let res = C::timeout(option.hard_ttl(), net.install_snapshot(req, option.clone())).await;

                let resp = match res {
                    Ok(outer_res) => match outer_res {
                        Ok(res) => res,
                        Err(err) => {
                            let err: RPCError<C, RaftError<C, InstallSnapshotError>> = err;

                            tracing::warn!(error=%err, "error sending InstallSnapshot RPC to target");

                            match err {
                                RPCError::Timeout(_) => {}
                                RPCError::Unreachable(_) => {}
                                RPCError::Network(_) => {}
                                RPCError::RemoteError(remote_err) => {
                                    //
                                    match remote_err.source {
                                        RaftError::Fatal(_) => {}
                                        RaftError::APIError(snapshot_err) => {
                                            //
                                            match snapshot_err {
                                                InstallSnapshotError::SnapshotMismatch(mismatch) => {
                                                    //
                                                    tracing::warn!(
                                                        mismatch = display(&mismatch),
                                                        "snapshot mismatch, reset offset and retry"
                                                    );
                                                    offset = 0;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                    },
                    Err(err) => {
                        tracing::warn!(error=%err, "timeout while sending InstallSnapshot RPC to target");
                        continue;
                    }
                };

                if resp.vote.as_ref_vote() > vote.as_ref_vote() {
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
        ) -> Result<Option<Snapshot<C>>, RaftError<C, InstallSnapshotError>> {
            let snapshot_id = &req.meta.snapshot_id;
            let snapshot_meta = req.meta.clone();
            let done = req.done;

            tracing::info!(req = display(&req), "{}", func_name!());

            let curr_id = streaming.as_ref().map(|s| s.snapshot_id());

            if curr_id != Some(snapshot_id) {
                if req.offset != 0 {
                    let mismatch = InstallSnapshotError::SnapshotMismatch(crate::error::SnapshotMismatch {
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
                    RaftError::Fatal(e.unwrap_fatal())
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

                data.shutdown().await.sto_write_snapshot(Some(snapshot_meta.signature()))?;

                tracing::info!("finished streaming snapshot: {:?}", snapshot_meta);
                return Ok(Some(Snapshot::new(snapshot_meta, data)));
            }

            Ok(None)
        }
    }

    impl<C> Streaming<C>
    where
        C: RaftTypeConfig,
        C::SnapshotData: tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin,
    {
        /// Receive a chunk of snapshot data.
        pub async fn receive(&mut self, req: InstallSnapshotRequest<C>) -> Result<bool, StorageError<C>> {
            // TODO: check id?

            // Always seek to the target offset if not an exact match.
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

            // Write the next segment & update offset.
            let res = self.snapshot_data.write_all(&req.data).await;
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
}

use std::future::Future;

use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::OptionalSend;
use crate::Raft;
use crate::RaftNetwork;
use crate::RaftTypeConfig;
use crate::SnapshotId;
use crate::error::InstallSnapshotError;
use crate::error::RaftError;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::network::RPCOption;
use crate::raft::InstallSnapshotRequest;
use crate::raft::SnapshotResponse;
use crate::storage::Snapshot;
use crate::type_config::alias::VoteOf;

/// Send and Receive snapshot by chunks.
pub struct Chunked {}

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
    /// It returns `Ready` if the caller decides to cancel this snapshot transmission.
    // TODO: consider removing dependency on RaftNetwork
    async fn send_snapshot<Net>(
        net: &mut Net,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>>
    where
        Net: RaftNetwork<C> + ?Sized;

    /// Receive a chunk of snapshot. If the snapshot is done receiving, return the snapshot.
    ///
    /// This method provides a default implementation for chunk-based snapshot transport
    /// and requires the caller to provide two things:
    ///
    /// - The receiving state `streaming` is maintained by the caller.
    /// - And it depends on `Raft::begin_receiving_snapshot()` to create a `SnapshotData` for
    ///   receiving data.
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
    ) -> Result<Option<Snapshot<C>>, RaftError<C, InstallSnapshotError>>;
}

/// The Raft node is streaming in a snapshot from the leader.
#[since(version = "0.10.0", change = "SnapshotData without Box")]
pub struct Streaming<C>
where C: RaftTypeConfig
{
    /// The offset of the last byte written to the snapshot.
    #[cfg_attr(not(feature = "tokio-rt"), allow(dead_code))]
    // This field will only be read when feature tokio-rt is on
    offset: u64,

    /// The ID of the snapshot being written.
    snapshot_id: SnapshotId,

    /// A handle to the snapshot writer.
    snapshot_data: C::SnapshotData,
}

impl<C> Streaming<C>
where C: RaftTypeConfig
{
    #[since(version = "0.10.0", change = "SnapshotData without Box")]
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

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::time::Duration;

    use crate::RaftNetwork;
    use crate::RaftTypeConfig;
    use crate::StoredMembership;
    use crate::Vote;
    use crate::engine::testing::UTConfig;
    use crate::error::InstallSnapshotError;
    use crate::error::RPCError;
    use crate::error::RaftError;
    use crate::error::SnapshotMismatch;
    use crate::network::RPCOption;
    use crate::network::snapshot_transport::Chunked;
    use crate::network::snapshot_transport::SnapshotTransport;
    use crate::raft::AppendEntriesRequest;
    use crate::raft::AppendEntriesResponse;
    use crate::raft::InstallSnapshotRequest;
    use crate::raft::InstallSnapshotResponse;
    use crate::raft::VoteRequest;
    use crate::raft::VoteResponse;
    use crate::storage::Snapshot;
    use crate::storage::SnapshotMeta;

    struct Network {
        received_offset: Vec<u64>,
        match_cnt: u64,
    }

    impl<C> RaftNetwork<C> for Network
    where C: RaftTypeConfig<NodeId = u64>
    {
        async fn append_entries(
            &mut self,
            _rpc: AppendEntriesRequest<C>,
            _option: RPCOption,
        ) -> Result<AppendEntriesResponse<C>, RPCError<C, RaftError<C>>> {
            unimplemented!()
        }

        async fn vote(
            &mut self,
            _rpc: VoteRequest<C>,
            _option: RPCOption,
        ) -> Result<VoteResponse<C>, RPCError<C, RaftError<C>>> {
            unimplemented!()
        }

        async fn install_snapshot(
            &mut self,
            rpc: InstallSnapshotRequest<C>,
            _option: RPCOption,
        ) -> Result<InstallSnapshotResponse<C>, RPCError<C, RaftError<C, InstallSnapshotError>>> {
            // A fake implementation to test the Chunked::send_snapshot.

            self.received_offset.push(rpc.offset);

            // For the second last time, return a mismatch error.
            // Then return Ok for the reset of the time.
            self.match_cnt = self.match_cnt.saturating_sub(1);
            if self.match_cnt == 1 {
                let mismatch = SnapshotMismatch {
                    expect: crate::SnapshotSegmentId {
                        id: rpc.meta.snapshot_id.clone(),
                        offset: 0,
                    },
                    got: crate::SnapshotSegmentId {
                        id: rpc.meta.snapshot_id.clone(),
                        offset: rpc.offset,
                    },
                };
                let err = RaftError::APIError(InstallSnapshotError::SnapshotMismatch(mismatch));
                Err(RPCError::RemoteError(crate::error::RemoteError::new(0, err)))
            } else {
                Ok(InstallSnapshotResponse { vote: rpc.vote })
            }
        }
    }

    /// Test that `Chunked` should reset the offset to 0 to re-send all data
    /// if a [`SnapshotMismatch`] error is received.
    #[tokio::test]
    async fn test_chunked_reset_offset_if_snapshot_id_mismatch() {
        let mut net = Network {
            received_offset: vec![],
            // When match_cnt == 1, return a mismatch error.
            // For other times, return Ok.
            match_cnt: 4,
        };

        let mut opt = RPCOption::new(Duration::from_millis(100));
        opt.snapshot_chunk_size = Some(1);
        let cancel = futures::future::pending();

        Chunked::send_snapshot(
            &mut net,
            Vote::new(1, 0),
            Snapshot::<UTConfig>::new(
                SnapshotMeta {
                    last_log_id: None,
                    last_membership: StoredMembership::default(),
                    snapshot_id: "1-1-1-1".to_string(),
                },
                Cursor::new(vec![1, 2, 3]),
            ),
            cancel,
            opt,
        )
        .await
        .unwrap();

        assert_eq!(net.received_offset, vec![0, 1, 2, 0, 1, 2]);
    }
}
