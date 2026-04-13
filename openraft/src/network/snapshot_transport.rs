//! Provide a default chunked snapshot transport implementation for SnapshotData that implements
//! AsyncWrite + AsyncRead + AsyncSeek + Unpin.

use std::future::Future;
use std::io::SeekFrom;
use std::time::Duration;

use anyerror::AnyError;
use futures::FutureExt;
use openraft_macros::add_async_trait;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;

use crate::error::Fatal;
use crate::error::InstallSnapshotError;
use crate::error::NetworkError;
use crate::error::RPCError;
use crate::error::RaftError;
use crate::error::RemoteError;
use crate::error::ReplicationClosed;
use crate::error::StreamingError;
use crate::network::Backoff;
use crate::network::RPCOption;
use crate::raft::InstallSnapshotRequest;
use crate::raft::SnapshotResponse;
use crate::type_config::TypeConfigExt;
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
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
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
    ) -> Result<Option<Snapshot<C>>, RaftError<C::NodeId, InstallSnapshotError>>;
}

/// Send and Receive snapshot by chunks.
pub struct Chunked {}

// Retry policy for chunked snapshot transport.
//
// `Timeout` and `Network` retries share a short exponential backoff local to
// this module: these errors typically clear within a packet-loss burst, and
// the caller's `Backoff` — intended for node-level unreachability — is too
// coarse for them.
//
// `Unreachable` uses the caller-supplied `RaftNetwork::backoff()` iterator
// because an unreachable target usually stays unreachable for seconds to
// minutes, a duration the application is better placed to pick.
//
// In all cases we cap at `SNAPSHOT_CHUNK_MAX_RETRIES` consecutive failures
// before surfacing the error; a successful chunk resets the counter and the
// backoff iterator so sporadic flakiness does not accumulate.
const SNAPSHOT_CHUNK_MAX_RETRIES: u64 = 5;
const SNAPSHOT_CHUNK_RETRY_BASE: Duration = Duration::from_millis(10);
const SNAPSHOT_CHUNK_RETRY_MAX: Duration = Duration::from_millis(200);

/// Fallback delay used when a caller-supplied [`Backoff`] iterator has been
/// exhausted. Matches the constant used by the default
/// [`RaftNetwork::backoff()`] implementation.
const SNAPSHOT_CHUNK_UNREACHABLE_FALLBACK: Duration = Duration::from_millis(500);

fn snapshot_chunk_retry_delay(consecutive_failures: u64) -> Duration {
    debug_assert!(consecutive_failures > 0);

    let shift = consecutive_failures.saturating_sub(1).min(4) as u32;
    let multiplier = 2u32.saturating_pow(shift);
    SNAPSHOT_CHUNK_RETRY_BASE.saturating_mul(multiplier).min(SNAPSHOT_CHUNK_RETRY_MAX)
}

/// This chunk based implementation requires `SnapshotData` to be `AsyncRead + AsyncSeek`.
impl<C: RaftTypeConfig> SnapshotTransport<C> for Chunked
where C::SnapshotData: tokio::io::AsyncRead + tokio::io::AsyncWrite + tokio::io::AsyncSeek + Unpin
{
    async fn send_snapshot<Net>(
        net: &mut Net,
        vote: Vote<C::NodeId>,
        mut snapshot: Snapshot<C>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>>
    where
        Net: RaftNetwork<C> + ?Sized,
    {
        let subject_verb = || (ErrorSubject::Snapshot(Some(snapshot.meta.signature())), ErrorVerb::Read);

        let mut offset = 0;
        let end = snapshot.snapshot.seek(SeekFrom::End(0)).await.sto_res(subject_verb)?;
        let mut consecutive_failures = 0;
        let mut unreachable_backoff = None::<Backoff>;

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
                Err(outer_err) => {
                    // Outer `hard_ttl` expired before `install_snapshot` returned. Don't retry
                    // here: the replication layer drives the next attempt on its own timer with
                    // a fresh snapshot, while looping here just stacks in-flight RPCs under the
                    // same deadline.
                    tracing::warn!(error=%outer_err, "InstallSnapshot RPC timed out");
                    let any_err = AnyError::error(format!("InstallSnapshot RPC timed out: {outer_err}"));
                    return Err(NetworkError::new(&any_err).into());
                }
                Ok(Ok(resp)) => {
                    consecutive_failures = 0;
                    unreachable_backoff = None;
                    resp
                }
                Ok(Err(err)) => {
                    tracing::warn!(error=%err, "error sending InstallSnapshot RPC");

                    // Handle terminal and stream-reset variants up front — they skip the retry
                    // budget.
                    let err = match err {
                        RPCError::PayloadTooLarge(payload) => {
                            // Retrying the same chunk at the same size cannot make progress;
                            // the append-entries shrink path in `ReplicationCore` is not
                            // reusable here yet.
                            let any_err = AnyError::error(format!("snapshot chunk rejected as too large: {payload}"));
                            return Err(NetworkError::new(&any_err).into());
                        }
                        RPCError::RemoteError(RemoteError {
                            target,
                            target_node,
                            source: RaftError::Fatal(fatal),
                        }) => {
                            return Err(RemoteError {
                                target,
                                target_node,
                                source: fatal,
                            }
                            .into());
                        }
                        RPCError::RemoteError(RemoteError {
                            source: RaftError::APIError(InstallSnapshotError::SnapshotMismatch(mismatch)),
                            ..
                        }) => {
                            tracing::warn!(
                                mismatch = display(&mismatch),
                                "snapshot mismatch, reset offset and retry"
                            );
                            offset = 0;
                            consecutive_failures = 0;
                            unreachable_backoff = None;
                            continue;
                        }
                        err @ (RPCError::Timeout(_) | RPCError::Network(_) | RPCError::Unreachable(_)) => err,
                    };

                    // Strict rule: any transient error that exceeds the budget returns
                    // immediately, regardless of which variant it is.
                    consecutive_failures += 1;
                    if consecutive_failures >= SNAPSHOT_CHUNK_MAX_RETRIES {
                        return Err(match err {
                            RPCError::Timeout(e) => e.into(),
                            RPCError::Network(e) => e.into(),
                            RPCError::Unreachable(e) => e.into(),
                            _ => unreachable!("non-transient variants handled above"),
                        });
                    }

                    let delay = if matches!(err, RPCError::Unreachable(_)) {
                        unreachable_backoff
                            .get_or_insert_with(|| net.backoff())
                            .next()
                            .unwrap_or(SNAPSHOT_CHUNK_UNREACHABLE_FALLBACK)
                    } else {
                        snapshot_chunk_retry_delay(consecutive_failures)
                    };

                    C::sleep(delay).await;
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
    ) -> Result<Option<Snapshot<C>>, RaftError<C::NodeId, InstallSnapshotError>> {
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

#[cfg(feature = "generic-snapshot-data")]
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::future::Future;
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    use anyerror::AnyError;
    use tokio::time::sleep;

    use super::SNAPSHOT_CHUNK_MAX_RETRIES;
    use crate::engine::testing::UTConfig;
    use crate::error::Fatal;
    use crate::error::InstallSnapshotError;
    use crate::error::NetworkError;
    use crate::error::PayloadTooLarge;
    use crate::error::RPCError;
    use crate::error::RaftError;
    use crate::error::RemoteError;
    use crate::error::ReplicationClosed;
    use crate::error::SnapshotMismatch;
    use crate::error::StreamingError;
    use crate::error::Timeout;
    use crate::error::Unreachable;
    use crate::network::snapshot_transport::Chunked;
    use crate::network::snapshot_transport::SnapshotTransport;
    use crate::network::Backoff;
    use crate::network::RPCOption;
    use crate::raft::AppendEntriesRequest;
    use crate::raft::AppendEntriesResponse;
    use crate::raft::InstallSnapshotRequest;
    use crate::raft::InstallSnapshotResponse;
    use crate::raft::SnapshotResponse;
    use crate::raft::VoteRequest;
    use crate::raft::VoteResponse;
    use crate::OptionalSend;
    use crate::RPCTypes;
    use crate::RaftNetwork;
    use crate::RaftTypeConfig;
    use crate::Snapshot;
    use crate::SnapshotMeta;
    use crate::StoredMembership;
    use crate::Vote;

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
        ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn vote(
            &mut self,
            _rpc: VoteRequest<C::NodeId>,
            _option: RPCOption,
        ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn full_snapshot(
            &mut self,
            _vote: Vote<C::NodeId>,
            _snapshot: Snapshot<C>,
            _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
            _option: RPCOption,
        ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
            unimplemented!()
        }

        async fn install_snapshot(
            &mut self,
            rpc: InstallSnapshotRequest<C>,
            _option: RPCOption,
        ) -> Result<
            InstallSnapshotResponse<C::NodeId>,
            RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
        > {
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
                Err(RPCError::RemoteError(RemoteError::new(0, err)))
            } else {
                Ok(InstallSnapshotResponse { vote: rpc.vote })
            }
        }
    }

    /// Test that `Chunked` should reset the offset to 0 to re-send all data,
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
                Box::new(Cursor::new(vec![1, 2, 3])),
            ),
            cancel,
            opt,
        )
        .await
        .unwrap();

        assert_eq!(net.received_offset, vec![0, 1, 2, 0, 1, 2]);
    }

    struct RetryNetwork {
        received_offset: Vec<u64>,
        fail_offset: u64,
        remaining_network_failures: u64,
    }

    impl<C> RaftNetwork<C> for RetryNetwork
    where C: RaftTypeConfig<NodeId = u64>
    {
        async fn append_entries(
            &mut self,
            _rpc: AppendEntriesRequest<C>,
            _option: RPCOption,
        ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn vote(
            &mut self,
            _rpc: VoteRequest<C::NodeId>,
            _option: RPCOption,
        ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn full_snapshot(
            &mut self,
            _vote: Vote<C::NodeId>,
            _snapshot: Snapshot<C>,
            _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
            _option: RPCOption,
        ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
            unimplemented!()
        }

        async fn install_snapshot(
            &mut self,
            rpc: InstallSnapshotRequest<C>,
            _option: RPCOption,
        ) -> Result<
            InstallSnapshotResponse<C::NodeId>,
            RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
        > {
            self.received_offset.push(rpc.offset);

            if rpc.offset == self.fail_offset && self.remaining_network_failures > 0 {
                self.remaining_network_failures -= 1;
                let any_err = AnyError::error("inject snapshot chunk network error");
                return Err(RPCError::Network(NetworkError::new(&any_err)));
            }

            Ok(InstallSnapshotResponse { vote: rpc.vote })
        }
    }

    #[tokio::test]
    async fn test_chunked_retry_resumes_from_current_offset_after_network_error() {
        let mut net = RetryNetwork {
            received_offset: vec![],
            fail_offset: 1,
            remaining_network_failures: 1,
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
                    snapshot_id: "1-1-1-2".to_string(),
                },
                Box::new(Cursor::new(vec![1, 2, 3])),
            ),
            cancel,
            opt,
        )
        .await
        .unwrap();

        assert_eq!(net.received_offset, vec![0, 1, 1, 2]);
    }

    #[tokio::test]
    async fn test_chunked_retry_budget_bails_out_after_consecutive_network_errors() {
        let mut net = RetryNetwork {
            received_offset: vec![],
            fail_offset: 1,
            remaining_network_failures: u64::MAX,
        };

        let mut opt = RPCOption::new(Duration::from_millis(100));
        opt.snapshot_chunk_size = Some(1);
        let cancel = futures::future::pending();

        let err = Chunked::send_snapshot(
            &mut net,
            Vote::new(1, 0),
            Snapshot::<UTConfig>::new(
                SnapshotMeta {
                    last_log_id: None,
                    last_membership: StoredMembership::default(),
                    snapshot_id: "1-1-1-3".to_string(),
                },
                Box::new(Cursor::new(vec![1, 2, 3])),
            ),
            cancel,
            opt,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, StreamingError::Network(_)));
        let mut expected = vec![0];
        expected.extend(std::iter::repeat_n(1, SNAPSHOT_CHUNK_MAX_RETRIES as usize));
        assert_eq!(net.received_offset, expected);
    }

    struct SlowNetwork {
        received_offset: Vec<u64>,
        delay: Duration,
    }

    impl<C> RaftNetwork<C> for SlowNetwork
    where C: RaftTypeConfig<NodeId = u64>
    {
        async fn append_entries(
            &mut self,
            _rpc: AppendEntriesRequest<C>,
            _option: RPCOption,
        ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn vote(
            &mut self,
            _rpc: VoteRequest<C::NodeId>,
            _option: RPCOption,
        ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn full_snapshot(
            &mut self,
            _vote: Vote<C::NodeId>,
            _snapshot: Snapshot<C>,
            _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
            _option: RPCOption,
        ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
            unimplemented!()
        }

        async fn install_snapshot(
            &mut self,
            rpc: InstallSnapshotRequest<C>,
            _option: RPCOption,
        ) -> Result<
            InstallSnapshotResponse<C::NodeId>,
            RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
        > {
            self.received_offset.push(rpc.offset);
            sleep(self.delay).await;
            Ok(InstallSnapshotResponse { vote: rpc.vote })
        }
    }

    #[tokio::test]
    async fn test_chunked_outer_timeout_does_not_retry() {
        let mut net = SlowNetwork {
            received_offset: vec![],
            delay: Duration::from_millis(20),
        };

        let mut opt = RPCOption::new(Duration::from_millis(1));
        opt.snapshot_chunk_size = Some(1);
        let cancel = futures::future::pending();

        let err = Chunked::send_snapshot(
            &mut net,
            Vote::new(1, 0),
            Snapshot::<UTConfig>::new(
                SnapshotMeta {
                    last_log_id: None,
                    last_membership: StoredMembership::default(),
                    snapshot_id: "1-1-1-4".to_string(),
                },
                Box::new(Cursor::new(vec![1, 2, 3])),
            ),
            cancel,
            opt,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, StreamingError::Network(_)));
        assert_eq!(net.received_offset, vec![0]);
    }

    struct PayloadTooLargeNetwork {
        received_offset: Vec<u64>,
    }

    impl<C> RaftNetwork<C> for PayloadTooLargeNetwork
    where C: RaftTypeConfig<NodeId = u64>
    {
        async fn append_entries(
            &mut self,
            _rpc: AppendEntriesRequest<C>,
            _option: RPCOption,
        ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn vote(
            &mut self,
            _rpc: VoteRequest<C::NodeId>,
            _option: RPCOption,
        ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn full_snapshot(
            &mut self,
            _vote: Vote<C::NodeId>,
            _snapshot: Snapshot<C>,
            _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
            _option: RPCOption,
        ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
            unimplemented!()
        }

        async fn install_snapshot(
            &mut self,
            rpc: InstallSnapshotRequest<C>,
            _option: RPCOption,
        ) -> Result<
            InstallSnapshotResponse<C::NodeId>,
            RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
        > {
            self.received_offset.push(rpc.offset);
            Err(RPCError::PayloadTooLarge(PayloadTooLarge::new_bytes_hint(1)))
        }
    }

    #[tokio::test]
    async fn test_chunked_payload_too_large_fails_fast() {
        let mut net = PayloadTooLargeNetwork {
            received_offset: vec![],
        };

        let mut opt = RPCOption::new(Duration::from_millis(100));
        opt.snapshot_chunk_size = Some(1);
        let cancel = futures::future::pending();

        let err = Chunked::send_snapshot(
            &mut net,
            Vote::new(1, 0),
            Snapshot::<UTConfig>::new(
                SnapshotMeta {
                    last_log_id: None,
                    last_membership: StoredMembership::default(),
                    snapshot_id: "1-1-1-5".to_string(),
                },
                Box::new(Cursor::new(vec![1, 2, 3])),
            ),
            cancel,
            opt,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, StreamingError::Network(_)));
        assert_eq!(net.received_offset, vec![0], "must not retry the same chunk");
    }

    struct FatalNetwork {
        received_offset: Vec<u64>,
    }

    impl<C> RaftNetwork<C> for FatalNetwork
    where C: RaftTypeConfig<NodeId = u64>
    {
        async fn append_entries(
            &mut self,
            _rpc: AppendEntriesRequest<C>,
            _option: RPCOption,
        ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn vote(
            &mut self,
            _rpc: VoteRequest<C::NodeId>,
            _option: RPCOption,
        ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn full_snapshot(
            &mut self,
            _vote: Vote<C::NodeId>,
            _snapshot: Snapshot<C>,
            _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
            _option: RPCOption,
        ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
            unimplemented!()
        }

        async fn install_snapshot(
            &mut self,
            rpc: InstallSnapshotRequest<C>,
            _option: RPCOption,
        ) -> Result<
            InstallSnapshotResponse<C::NodeId>,
            RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
        > {
            self.received_offset.push(rpc.offset);
            Err(RPCError::RemoteError(RemoteError::new(
                2,
                RaftError::Fatal(Fatal::Panicked),
            )))
        }
    }

    #[tokio::test]
    async fn test_chunked_remote_fatal_is_propagated() {
        let mut net = FatalNetwork {
            received_offset: vec![],
        };

        let mut opt = RPCOption::new(Duration::from_millis(100));
        opt.snapshot_chunk_size = Some(1);
        let cancel = futures::future::pending();

        let err = Chunked::send_snapshot(
            &mut net,
            Vote::new(1, 0),
            Snapshot::<UTConfig>::new(
                SnapshotMeta {
                    last_log_id: None,
                    last_membership: StoredMembership::default(),
                    snapshot_id: "1-1-1-6".to_string(),
                },
                Box::new(Cursor::new(vec![1, 2, 3])),
            ),
            cancel,
            opt,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, StreamingError::RemoteError(_)),
            "remote Fatal must surface, got {err:?}"
        );
        assert_eq!(net.received_offset, vec![0], "remote Fatal must not be retried");
    }

    struct UnreachableNetwork {
        received_offset: Vec<u64>,
        /// Offsets at which one `Unreachable` error should be injected (consumed on first hit).
        unreachable_at: Vec<u64>,
        backoff_calls: Arc<Mutex<u64>>,
    }

    impl<C> RaftNetwork<C> for UnreachableNetwork
    where C: RaftTypeConfig<NodeId = u64>
    {
        async fn append_entries(
            &mut self,
            _rpc: AppendEntriesRequest<C>,
            _option: RPCOption,
        ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn vote(
            &mut self,
            _rpc: VoteRequest<C::NodeId>,
            _option: RPCOption,
        ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn full_snapshot(
            &mut self,
            _vote: Vote<C::NodeId>,
            _snapshot: Snapshot<C>,
            _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
            _option: RPCOption,
        ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
            unimplemented!()
        }

        fn backoff(&self) -> Backoff {
            *self.backoff_calls.lock().unwrap() += 1;
            // Use a short, infinite iterator so the test is fast but the iterator never exhausts.
            Backoff::new(std::iter::repeat(Duration::from_millis(1)))
        }

        async fn install_snapshot(
            &mut self,
            rpc: InstallSnapshotRequest<C>,
            _option: RPCOption,
        ) -> Result<
            InstallSnapshotResponse<C::NodeId>,
            RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
        > {
            self.received_offset.push(rpc.offset);

            if let Some(idx) = self.unreachable_at.iter().position(|&o| o == rpc.offset) {
                self.unreachable_at.remove(idx);
                let any_err = AnyError::error("target unreachable");
                return Err(RPCError::Unreachable(Unreachable::new(&any_err)));
            }
            Ok(InstallSnapshotResponse { vote: rpc.vote })
        }
    }

    /// `Unreachable` errors pull the next delay from `net.backoff()`, and the cached iterator
    /// is dropped on the next successful chunk so a fresh outage gets a fresh backoff.
    #[tokio::test]
    async fn test_chunked_retry_on_unreachable_uses_caller_backoff() {
        let backoff_calls = Arc::new(Mutex::new(0u64));
        let mut net = UnreachableNetwork {
            received_offset: vec![],
            // Two separate outages: one at offset 1, one at offset 2.
            unreachable_at: vec![1, 2],
            backoff_calls: backoff_calls.clone(),
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
                    snapshot_id: "1-1-1-7".to_string(),
                },
                Box::new(Cursor::new(vec![1, 2, 3])),
            ),
            cancel,
            opt,
        )
        .await
        .unwrap();

        assert_eq!(net.received_offset, vec![0, 1, 1, 2, 2]);
        assert_eq!(
            *backoff_calls.lock().unwrap(),
            2,
            "net.backoff() must be called once per outage — the cached iterator is dropped on success"
        );
    }

    struct BurstyNetwork {
        received_offset: Vec<u64>,
        /// For each offset, how many `Network` failures remain to inject before it succeeds.
        fails_remaining_per_offset: HashMap<u64, u64>,
    }

    impl<C> RaftNetwork<C> for BurstyNetwork
    where C: RaftTypeConfig<NodeId = u64>
    {
        async fn append_entries(
            &mut self,
            _rpc: AppendEntriesRequest<C>,
            _option: RPCOption,
        ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn vote(
            &mut self,
            _rpc: VoteRequest<C::NodeId>,
            _option: RPCOption,
        ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn full_snapshot(
            &mut self,
            _vote: Vote<C::NodeId>,
            _snapshot: Snapshot<C>,
            _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
            _option: RPCOption,
        ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
            unimplemented!()
        }

        async fn install_snapshot(
            &mut self,
            rpc: InstallSnapshotRequest<C>,
            _option: RPCOption,
        ) -> Result<
            InstallSnapshotResponse<C::NodeId>,
            RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
        > {
            self.received_offset.push(rpc.offset);

            let remaining = self.fails_remaining_per_offset.entry(rpc.offset).or_insert(0);
            if *remaining > 0 {
                *remaining -= 1;
                let any_err = AnyError::error("inject network error");
                return Err(RPCError::Network(NetworkError::new(&any_err)));
            }
            Ok(InstallSnapshotResponse { vote: rpc.vote })
        }
    }

    /// A successful chunk resets `consecutive_failures`, so several bursts of
    /// `MAX_RETRIES - 1` failures in a row succeed even though the total number of
    /// failures exceeds the budget.
    #[tokio::test]
    async fn test_chunked_success_resets_retry_budget() {
        let burst = SNAPSHOT_CHUNK_MAX_RETRIES - 1;
        let mut fails = HashMap::new();
        fails.insert(0, burst);
        fails.insert(1, burst);

        let mut net = BurstyNetwork {
            received_offset: vec![],
            fails_remaining_per_offset: fails,
        };

        let mut opt = RPCOption::new(Duration::from_millis(100));
        opt.snapshot_chunk_size = Some(1);
        let cancel = futures::future::pending();

        // 2-byte snapshot = 2 chunks. Total failures injected = 2 * burst > MAX_RETRIES.
        Chunked::send_snapshot(
            &mut net,
            Vote::new(1, 0),
            Snapshot::<UTConfig>::new(
                SnapshotMeta {
                    last_log_id: None,
                    last_membership: StoredMembership::default(),
                    snapshot_id: "1-1-1-8".to_string(),
                },
                Box::new(Cursor::new(vec![1, 2])),
            ),
            cancel,
            opt,
        )
        .await
        .unwrap();

        let attempts_per_offset = (burst + 1) as usize;
        let mut expected = Vec::new();
        expected.extend(std::iter::repeat_n(0u64, attempts_per_offset));
        expected.extend(std::iter::repeat_n(1u64, attempts_per_offset));
        assert_eq!(net.received_offset, expected);
    }

    #[derive(Clone, Copy)]
    enum TransientKind {
        Timeout,
        Network,
        Unreachable,
    }

    struct ScriptedErrorNetwork {
        received_offset: Vec<u64>,
        /// Responses to emit when `install_snapshot` is called at `fail_offset`, in order.
        script: Vec<TransientKind>,
        fail_offset: u64,
    }

    impl<C> RaftNetwork<C> for ScriptedErrorNetwork
    where C: RaftTypeConfig<NodeId = u64>
    {
        async fn append_entries(
            &mut self,
            _rpc: AppendEntriesRequest<C>,
            _option: RPCOption,
        ) -> Result<AppendEntriesResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn vote(
            &mut self,
            _rpc: VoteRequest<C::NodeId>,
            _option: RPCOption,
        ) -> Result<VoteResponse<C::NodeId>, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
            unimplemented!()
        }

        async fn full_snapshot(
            &mut self,
            _vote: Vote<C::NodeId>,
            _snapshot: Snapshot<C>,
            _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
            _option: RPCOption,
        ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
            unimplemented!()
        }

        fn backoff(&self) -> Backoff {
            Backoff::new(std::iter::repeat(Duration::from_millis(1)))
        }

        async fn install_snapshot(
            &mut self,
            rpc: InstallSnapshotRequest<C>,
            _option: RPCOption,
        ) -> Result<
            InstallSnapshotResponse<C::NodeId>,
            RPCError<C::NodeId, C::Node, RaftError<C::NodeId, InstallSnapshotError>>,
        > {
            self.received_offset.push(rpc.offset);

            if rpc.offset != self.fail_offset || self.script.is_empty() {
                return Ok(InstallSnapshotResponse { vote: rpc.vote });
            }

            Err(match self.script.remove(0) {
                TransientKind::Timeout => RPCError::Timeout(Timeout {
                    action: RPCTypes::InstallSnapshot,
                    id: 0,
                    target: 1,
                    timeout: Duration::from_millis(50),
                }),
                TransientKind::Network => {
                    let any_err = AnyError::error("inject network");
                    RPCError::Network(NetworkError::new(&any_err))
                }
                TransientKind::Unreachable => {
                    let any_err = AnyError::error("inject unreachable");
                    RPCError::Unreachable(Unreachable::new(&any_err))
                }
            })
        }
    }

    /// The retry budget counts every transient variant uniformly. Mixing `Timeout`,
    /// `Network`, and `Unreachable` errors still bails out at exactly
    /// `SNAPSHOT_CHUNK_MAX_RETRIES` consecutive failures.
    #[tokio::test]
    async fn test_chunked_retry_budget_is_universal_across_transient_variants() {
        let script = vec![
            TransientKind::Timeout,
            TransientKind::Network,
            TransientKind::Unreachable,
            TransientKind::Timeout,
            TransientKind::Network,
        ];
        assert_eq!(script.len() as u64, SNAPSHOT_CHUNK_MAX_RETRIES);

        let mut net = ScriptedErrorNetwork {
            received_offset: vec![],
            script,
            fail_offset: 1,
        };

        let mut opt = RPCOption::new(Duration::from_millis(100));
        opt.snapshot_chunk_size = Some(1);
        let cancel = futures::future::pending();

        let err = Chunked::send_snapshot(
            &mut net,
            Vote::new(1, 0),
            Snapshot::<UTConfig>::new(
                SnapshotMeta {
                    last_log_id: None,
                    last_membership: StoredMembership::default(),
                    snapshot_id: "1-1-1-9".to_string(),
                },
                Box::new(Cursor::new(vec![1, 2, 3])),
            ),
            cancel,
            opt,
        )
        .await
        .unwrap_err();

        // The last scripted variant is Network, so the returned StreamingError variant matches.
        assert!(
            matches!(err, StreamingError::Network(_)),
            "last variant was Network, got {err:?}"
        );

        let mut expected = vec![0];
        expected.extend(std::iter::repeat_n(1u64, SNAPSHOT_CHUNK_MAX_RETRIES as usize));
        assert_eq!(net.received_offset, expected);
    }
}
