//! Replication stream.

use std::io::SeekFrom;
use std::sync::Arc;

use futures::future::FutureExt;
use serde::Deserialize;
use serde::Serialize;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeek;
use tokio::io::AsyncSeekExt;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::interval;
use tokio::time::timeout;
use tokio::time::Duration;
use tokio::time::Interval;
use tracing::Instrument;
use tracing::Span;

use crate::config::Config;
use crate::config::SnapshotPolicy;
use crate::error::RaftResult;
use crate::raft::AppendEntriesRequest;
use crate::raft::Entry;
use crate::raft::InstallSnapshotRequest;
use crate::storage::Snapshot;
use crate::AppData;
use crate::AppDataResponse;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
use crate::RaftError;
use crate::RaftNetwork;
use crate::RaftStorage;

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplicationMetrics {
    pub matched: LogId,
}

/// The public handle to a spawned replication stream.
pub(crate) struct ReplicationStream<D: AppData> {
    /// The spawn handle the `ReplicationCore` task.
    // pub handle: JoinHandle<()>,
    /// The channel used for communicating with the replication task.
    pub repl_tx: mpsc::UnboundedSender<(RaftEvent<D>, Span)>,
}

impl<D: AppData> ReplicationStream<D> {
    /// Create a new replication stream for the target peer.
    pub(crate) fn new<R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>>(
        id: NodeId,
        target: NodeId,
        term: u64,
        config: Arc<Config>,
        last_log: LogId,
        commit_index: u64,
        network: Arc<N>,
        storage: Arc<S>,
        replication_tx: mpsc::UnboundedSender<(ReplicaEvent<S::SnapshotData>, Span)>,
    ) -> Self {
        ReplicationCore::spawn(
            id,
            target,
            term,
            config,
            last_log,
            commit_index,
            network,
            storage,
            replication_tx,
        )
    }
}

/// A task responsible for sending replication events to a target follower in the Raft cluster.
///
/// NOTE: we do not stack replication requests to targets because this could result in
/// out-of-order delivery. We always buffer until we receive a success response, then send the
/// next payload from the buffer.
struct ReplicationCore<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    //////////////////////////////////////////////////////////////////////////
    // Static Fields /////////////////////////////////////////////////////////
    /// The ID of this Raft node.
    id: NodeId,
    /// The ID of the target Raft node which replication events are to be sent to.
    target: NodeId,
    /// The current term, which will never change during the lifetime of this task.
    term: u64,

    /// A channel for sending events to the Raft node.
    raft_core_tx: mpsc::UnboundedSender<(ReplicaEvent<S::SnapshotData>, Span)>,

    /// A channel for receiving events from the Raft node.
    repl_rx: mpsc::UnboundedReceiver<(RaftEvent<D>, Span)>,

    /// The `RaftNetwork` interface.
    network: Arc<N>,

    /// The `RaftStorage` interface.
    storage: Arc<S>,

    /// The Raft's runtime config.
    config: Arc<Config>,

    marker_r: std::marker::PhantomData<R>,

    //////////////////////////////////////////////////////////////////////////
    // Dynamic Fields ////////////////////////////////////////////////////////
    /// The target state of this replication stream.
    target_state: TargetReplState,

    /// The index of the log entry to most recently be appended to the log by the leader.
    last_log_index: u64,
    /// The index of the highest log entry which is known to be committed in the cluster.
    commit_index: u64,

    /// The last know log to be successfully replicated on the target.
    ///
    /// This Raft implementation also uses a _conflict optimization_ pattern for reducing the
    /// number of RPCs which need to be sent back and forth between a peer which is lagging
    /// behind. This is defined in §5.3.
    /// This will be initialized to the leader's (last_log_term, last_log_index), and will be updated as
    /// replication proceeds.
    matched: LogId,

    /// The heartbeat interval for ensuring that heartbeats are always delivered in a timely fashion.
    heartbeat: Interval,

    /// The timeout for sending snapshot segment.
    install_snapshot_timeout: Duration,
}

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> ReplicationCore<D, R, N, S> {
    /// Spawn a new replication task for the target node.
    pub(self) fn spawn(
        id: NodeId,
        target: NodeId,
        term: u64,
        config: Arc<Config>,
        last_log: LogId,
        commit_index: u64,
        network: Arc<N>,
        storage: Arc<S>,
        raft_core_tx: mpsc::UnboundedSender<(ReplicaEvent<S::SnapshotData>, Span)>,
    ) -> ReplicationStream<D> {
        // other component to ReplicationStream
        let (repl_tx, repl_rx) = mpsc::unbounded_channel();
        let heartbeat_timeout = Duration::from_millis(config.heartbeat_interval);
        let install_snapshot_timeout = Duration::from_millis(config.install_snapshot_timeout);

        let this = Self {
            id,
            target,
            term,
            network,
            storage,
            config,
            marker_r: std::marker::PhantomData,
            target_state: TargetReplState::LineRate,
            last_log_index: last_log.index,
            commit_index,
            matched: LogId { term: 0, index: 0 },
            raft_core_tx,
            repl_rx,
            heartbeat: interval(heartbeat_timeout),
            install_snapshot_timeout,
        };

        let _handle = tokio::spawn(this.main().instrument(tracing::debug_span!("spawn")));

        ReplicationStream {
            // handle,
            repl_tx,
        }
    }

    #[tracing::instrument(level="trace", skip(self), fields(id=self.id, target=self.target, cluster=%self.config.cluster_name))]
    async fn main(mut self) {
        // Perform an initial heartbeat.
        self.send_append_entries().await;

        // Proceed to the replication stream's inner loop.
        loop {
            match &self.target_state {
                TargetReplState::LineRate => self.line_rate_loop().await,
                TargetReplState::Snapshotting => self.replicate_snapshot().await,
                TargetReplState::Shutdown => return,
            }
        }
    }

    /// Send an AppendEntries RPC to the target.
    ///
    /// This request will timeout if no response is received within the
    /// configured heartbeat interval.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn send_append_entries(&mut self) {
        let start = self.matched.index + 1;
        let end = self.last_log_index + 1;

        let chunk_size = std::cmp::min(self.config.max_payload_entries, end - start);
        let end = start + chunk_size;

        let logs;
        if chunk_size == 0 {
            // just a heartbeat
            logs = vec![];
        } else {
            let res = self.load_log_entries(start, end).await;

            let logs_opt = match res {
                Ok(x) => x,
                Err(err) => {
                    tracing::warn!(error=%err, "storage.get_log_entries, [{}, {})", start,start+chunk_size as u64);
                    self.set_target_state(TargetReplState::Shutdown);
                    let _ = self.raft_core_tx.send((ReplicaEvent::Shutdown, tracing::debug_span!("CH")));
                    return;
                }
            };

            logs = match logs_opt {
                // state changed to snapshot
                None => return,
                Some(x) => x,
            };
        }

        let last_log_id = logs.last().map(|last| last.log_id);

        // Build the heartbeat frame to be sent to the follower.
        let payload = AppendEntriesRequest {
            term: self.term,
            leader_id: self.id,
            prev_log_id: self.matched,
            leader_commit: self.commit_index,
            entries: logs,
        };

        // Send the payload.
        tracing::debug!(
            "start sending append_entries, timeout: {:?}",
            self.config.heartbeat_interval
        );

        let res = timeout(
            Duration::from_millis(self.config.heartbeat_interval),
            self.network.send_append_entries(self.target, payload),
        )
        .await;

        let res = match res {
            Ok(outer_res) => match outer_res {
                Ok(res) => res,
                Err(err) => {
                    tracing::warn!(error=%err, "error sending AppendEntries RPC to target");
                    return;
                }
            },
            Err(err) => {
                tracing::warn!(error=%err, "timeout while sending AppendEntries RPC to target");
                return;
            }
        };

        tracing::debug!("append_entries last: {:?}", last_log_id);

        // Handle success conditions.
        if res.success {
            tracing::debug!("append_entries success: last: {:?}", last_log_id);
            // If this was a proper replication event (last index & term were provided), then update state.
            if let Some(log_id) = last_log_id {
                self.matched = log_id;
                self.update_matched();
            }
            return;
        }

        // Failed

        // Replication was not successful, if a newer term has been returned, revert to follower.
        if res.term > self.term {
            tracing::debug!({ res.term }, "append entries failed, reverting to follower");
            let _ = self.raft_core_tx.send((
                ReplicaEvent::RevertToFollower {
                    target: self.target,
                    term: res.term,
                },
                tracing::debug_span!("CH"),
            ));
            self.set_target_state(TargetReplState::Shutdown);
            return;
        }

        // Replication was not successful, handle conflict optimization record, else decrement `next_index`.
        let mut conflict = match res.conflict_opt {
            None => {
                panic!("append_entries failed but without a reason: {:?}", res);
            }
            Some(x) => x,
        };

        tracing::debug!(?conflict, res.term, "append entries failed, handling conflict opt");

        // If conflict index is 0, we will not be able to fetch that index from storage because
        // it will never exist. So instead, we just return, and accept the conflict data.
        if conflict.log_id.index == 0 {
            self.matched = LogId { term: 0, index: 0 };
            self.update_matched();

            return;
        }

        // The follower has more log and set the conflict.log_id to its last_log_id if:
        // - req.prev_log_id.index is 0
        // - req.prev_log_id.index is applied, in which case the follower does not know if the prev_log_id is valid.
        //
        // In such case, fake a conflict log_id that never matches the log term in local store, in order to not
        // update_matched().
        if conflict.log_id.index > self.last_log_index {
            conflict.log_id = LogId {
                term: 0,
                index: self.last_log_index,
            };
        }

        // Fetch the entry at conflict index and use the term specified there.
        let ent = self.storage.try_get_log_entry(conflict.log_id.index).await;
        let ent = match ent {
            Ok(x) => x,
            Err(err) => {
                tracing::error!(error=?err, "error fetching log entry due to returned AppendEntries RPC conflict_opt");
                self.set_target_state(TargetReplState::Shutdown);
                let _ = self.raft_core_tx.send((ReplicaEvent::Shutdown, tracing::debug_span!("CH")));
                return;
            }
        };

        let ent = match ent {
            Some(x) => x,
            None => {
                // This condition would only ever be reached if the log has been removed due to
                // log compaction (barring critical storage failure), so transition to snapshotting.
                self.set_target_state(TargetReplState::Snapshotting);
                return;
            }
        };

        let term = ent.log_id.term;

        // Next time try sending from the log at conflict.log_id.index.
        self.matched = ent.log_id;

        if term == conflict.log_id.term {
            self.update_matched();
        }
    }

    #[tracing::instrument(level = "debug", skip(self))]
    fn set_target_state(&mut self, state: TargetReplState) {
        self.target_state = state;
    }

    #[tracing::instrument(level = "debug", skip(self))]
    fn update_matched(&mut self) {
        tracing::debug!(target=%self.target, matched=%self.matched, "update_matched");

        let _ = self.raft_core_tx.send((
            ReplicaEvent::UpdateMatchIndex {
                target: self.target,
                matched: self.matched,
            },
            tracing::debug_span!("CH"),
        ));
    }

    /// Perform a check to see if this replication stream is lagging behind far enough that a
    /// snapshot is warranted.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(self) fn needs_snapshot(&self) -> bool {
        match &self.config.snapshot_policy {
            SnapshotPolicy::LogsSinceLast(threshold) => {
                let needs_snap =
                    self.commit_index.checked_sub(self.matched.index).map(|diff| diff >= *threshold).unwrap_or(false);

                tracing::trace!("snapshot needed: {}", needs_snap);
                needs_snap
            }
        }
    }

    /// Fully drain the channel coming in from the Raft node.
    pub(self) fn drain_raft_rx(&mut self, first: RaftEvent<D>, span: Span) {
        let mut event_opt = Some((first, span));
        let mut iters = 0;
        loop {
            // Just ensure we don't get stuck draining a REALLY hot replication feed.
            if iters > self.config.max_payload_entries {
                return;
            }

            // Unpack the event opt, else return if we don't have one to process.
            let (event, span) = match event_opt.take() {
                Some(event) => event,
                None => return,
            };

            let _ent = span.enter();

            // Process the event.
            match event {
                RaftEvent::UpdateCommitIndex { commit_index } => {
                    self.commit_index = commit_index;
                }

                RaftEvent::Replicate { entry, commit_index } => {
                    self.commit_index = commit_index;
                    self.last_log_index = entry.log_id.index;
                }

                RaftEvent::Terminate => {
                    self.set_target_state(TargetReplState::Shutdown);
                    return;
                }
            }

            // Attempt to unpack the next event for the next loop iteration.
            if let Some(event_span) = self.repl_rx.recv().now_or_never() {
                event_opt = event_span;
            }
            iters += 1;
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// The state of the replication stream.
#[derive(Debug, Eq, PartialEq)]
enum TargetReplState {
    /// The replication stream is running at line rate.
    LineRate,
    /// The replication stream is streaming a snapshot over to the target node.
    Snapshotting,
    /// The replication stream is shutting down.
    Shutdown,
}

/// An event from the Raft node.
pub(crate) enum RaftEvent<D: AppData> {
    Replicate {
        /// The new entry which needs to be replicated.
        ///
        /// This entry will always be the most recent entry to have been appended to the log, so its
        /// index is the new last_log_index value.
        entry: Arc<Entry<D>>,
        /// The index of the highest log entry which is known to be committed in the cluster.
        commit_index: u64,
    },
    /// A message from Raft indicating a new commit index value.
    UpdateCommitIndex {
        /// The index of the highest log entry which is known to be committed in the cluster.
        commit_index: u64,
    },
    Terminate,
}

/// An event coming from a replication stream.
pub(crate) enum ReplicaEvent<S>
where S: AsyncRead + AsyncSeek + Send + Unpin + 'static
{
    /// An event from a replication stream which updates the target node's match index.
    UpdateMatchIndex {
        /// The ID of the target node for which the match index is to be updated.
        target: NodeId,
        /// The log of the most recent log known to have been successfully replicated on the target.
        matched: LogId,
    },
    /// An event indicating that the Raft node needs to revert to follower state.
    RevertToFollower {
        /// The ID of the target node from which the new term was observed.
        target: NodeId,
        /// The new term observed.
        term: u64,
    },
    /// An event from a replication stream requesting snapshot info.
    NeedsSnapshot {
        /// The ID of the target node from which the event was sent.
        target: NodeId,
        /// The response channel for delivering the snapshot data.
        tx: oneshot::Sender<Snapshot<S>>,
    },
    /// Some critical error has taken place, and Raft needs to shutdown.
    Shutdown,
}

impl<S: AsyncRead + AsyncSeek + Send + Unpin + 'static> MessageSummary for ReplicaEvent<S> {
    fn summary(&self) -> String {
        match self {
            ReplicaEvent::UpdateMatchIndex {
                ref target,
                ref matched,
            } => {
                format!("UpdateMatchIndex: target: {}, matched: {}", target, matched)
            }
            ReplicaEvent::RevertToFollower { ref target, ref term } => {
                format!("RevertToFollower: target: {}, term: {}", target, term)
            }
            ReplicaEvent::NeedsSnapshot { ref target, .. } => {
                format!("NeedsSnapshot: target: {}", target)
            }
            ReplicaEvent::Shutdown => "Shutdown".to_string(),
        }
    }
}

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> ReplicationCore<D, R, N, S> {
    #[tracing::instrument(level = "trace", skip(self), fields(state = "line-rate"))]
    pub async fn line_rate_loop(&mut self) {
        loop {
            if self.target_state != TargetReplState::LineRate {
                return;
            }

            if self.needs_snapshot() {
                self.set_target_state(TargetReplState::Snapshotting);
                return;
            }

            if self.matched.index < self.last_log_index {
                self.send_append_entries().await;

                if self.target_state != TargetReplState::LineRate {
                    return;
                }

                continue;
            }

            let span = tracing::debug_span!("CHrx:LineRate");
            let _en = span.enter();

            tokio::select! {
                _ = self.heartbeat.tick() => {
                    self.send_append_entries().await;
                }

                event_span = self.repl_rx.recv() => {
                    match event_span {
                        Some((event, span)) => self.drain_raft_rx(event, span),
                        None => {
                            self.set_target_state(TargetReplState::Shutdown);
                        },
                    }
                }
            }
        }
    }

    /// Ensure there are no gaps in the outbound buffer due to transition from lagging.
    #[tracing::instrument(level = "debug", skip(self))]
    async fn load_log_entries(&mut self, start: u64, stop: u64) -> Result<Option<Vec<Entry<D>>>, RaftError> {
        // TODO(xp): get_log_entries() should return an error that is programmable readable.
        //           EntryNotFound
        let entries = match self.storage.get_log_entries(start..stop).await {
            Ok(entries) => entries,
            Err(err) => {
                // TODO non-EntryNotFound error should not shutdown raft core.
                tracing::info!(error=%err, "loading log entries, switch to snapshot replication");
                self.set_target_state(TargetReplState::Snapshotting);
                return Ok(None);
            }
        };

        tracing::debug!(entries=%entries.as_slice().summary(), "load_log_entries");

        let first = entries.first().map(|x| x.log_id.index);
        if first != Some(start) {
            tracing::info!(
                "entry {} to replicate not found, first: {:?}, switch to snapshot replication",
                start,
                first
            );
            self.set_target_state(TargetReplState::Snapshotting);
            return Ok(None);
        }

        Ok(Some(entries))
    }

    #[tracing::instrument(level = "trace", skip(self), fields(state = "snapshotting"))]
    pub async fn replicate_snapshot(&mut self) {
        let res = self.wait_for_snapshot().await;

        // TODO(xp): bad impl: res is error only when replication is closed.
        //           use specific error to describe these behavior.

        let snapshot = match res {
            Ok(x) => x,
            Err(e) => {
                tracing::error!(error=%e, "replication shutting down");
                return;
            }
        };

        if let Err(err) = self.stream_snapshot(snapshot).await {
            tracing::warn!(error=%err, "error streaming snapshot to target");
        }
    }

    /// Wait for a response from the storage layer for the current snapshot.
    ///
    /// If an error comes up during processing, this routine should simple be called again after
    /// issuing a new request to the storage layer.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn wait_for_snapshot(&mut self) -> Result<Snapshot<S::SnapshotData>, RaftError> {
        // Ask raft core for a snapshot.
        // - If raft core has a ready snapshot, it sends back through tx.
        // - Otherwise raft core starts a new task taking snapshot, and **close** `tx` when finished. Thus there has to
        //   be a loop.

        loop {
            // channel to communicate with raft-core
            let (tx, mut rx) = oneshot::channel();

            // TODO(xp): handle sending error.
            let _ = self.raft_core_tx.send((
                ReplicaEvent::NeedsSnapshot {
                    target: self.target,
                    tx,
                },
                tracing::debug_span!("CH"),
            ));

            let mut waiting_for_snapshot = true;

            while waiting_for_snapshot {
                tokio::select! {
                    _ = self.heartbeat.tick() => self.send_append_entries().await,

                    event_span = self.repl_rx.recv() =>  {
                        match event_span {

                            Some((event, span)) => self.drain_raft_rx(event, span),
                            None => {
                                tracing::error!("repl_rx is closed");
                                self.set_target_state(TargetReplState::Shutdown);
                                // TODO: make it two errors: ReplicationShutdown and CoreShutdown
                                return Err(RaftError::ShuttingDown);
                            }
                        }
                    },

                    res = &mut rx => {
                        match res {
                            Ok(snapshot) => {
                                return Ok(snapshot);
                            }
                            Err(_) => {
                                // TODO(xp): This channel is closed to notify an in progress snapshotting is completed.

                                tracing::info!("rx for waiting for snapshot is closed, may be snapshot is ready. re-send need-snapshot.");
                                waiting_for_snapshot = false;
                            },
                        }
                    },
                }
            }
        }
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn stream_snapshot(&mut self, mut snapshot: Snapshot<S::SnapshotData>) -> RaftResult<()> {
        let end = snapshot.snapshot.seek(SeekFrom::End(0)).await?;

        let mut offset = 0;

        let mut buf = Vec::with_capacity(self.config.snapshot_max_chunk_size as usize);

        loop {
            // Build the RPC.
            snapshot.snapshot.seek(SeekFrom::Start(offset)).await?;
            let n_read = snapshot.snapshot.read_buf(&mut buf).await?;

            let done = (offset + n_read as u64) == end; // If bytes read == 0, then we're done.
            let req = InstallSnapshotRequest {
                term: self.term,
                leader_id: self.id,
                meta: snapshot.meta.clone(),
                offset,
                data: Vec::from(&buf[..n_read]),
                done,
            };
            buf.clear();

            // Send the RPC over to the target.
            tracing::debug!(
                snapshot_size = req.data.len(),
                req.offset,
                end,
                req.done,
                "sending snapshot chunk"
            );

            let res = timeout(
                self.install_snapshot_timeout,
                self.network.send_install_snapshot(self.target, req),
            )
            .await;

            let res = match res {
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

            // Handle response conditions.
            if res.term > self.term {
                let _ = self.raft_core_tx.send((
                    ReplicaEvent::RevertToFollower {
                        target: self.target,
                        term: res.term,
                    },
                    tracing::debug_span!("CH"),
                ));
                self.set_target_state(TargetReplState::Shutdown);
                return Ok(());
            }

            // If we just sent the final chunk of the snapshot, then transition to lagging state.
            if done {
                self.set_target_state(TargetReplState::LineRate);

                tracing::debug!(
                    "done install snapshot: snapshot last_log_id: {}, matched: {}",
                    snapshot.meta.last_log_id,
                    self.matched,
                );

                if snapshot.meta.last_log_id > self.matched {
                    self.matched = snapshot.meta.last_log_id;
                    self.update_matched();
                }
                return Ok(());
            }

            // Everything is good, so update offset for sending the next chunk.
            offset += n_read as u64;

            // Check raft channel to ensure we are staying up-to-date, then loop.
            if let Some(Some((event, span))) = self.repl_rx.recv().now_or_never() {
                self.drain_raft_rx(event, span);
            }
        }
    }
}
