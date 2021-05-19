//! Replication stream.

use std::io::SeekFrom;
use std::sync::Arc;

use futures::future::FutureExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{interval, timeout, Duration, Interval};

use crate::config::{Config, SnapshotPolicy};
use crate::error::RaftResult;
use crate::raft::{AppendEntriesRequest, Entry, EntryPayload, InstallSnapshotRequest};
use crate::storage::CurrentSnapshotData;
use crate::{AppData, AppDataResponse, NodeId, RaftNetwork, RaftStorage};

/// The public handle to a spawned replication stream.
pub(crate) struct ReplicationStream<D: AppData> {
    /// The spawn handle the `ReplicationCore` task.
    pub handle: JoinHandle<()>,
    /// The channel used for communicating with the replication task.
    pub repltx: mpsc::UnboundedSender<RaftEvent<D>>,
}

impl<D: AppData> ReplicationStream<D> {
    /// Create a new replication stream for the target peer.
    pub(crate) fn new<R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>>(
        id: NodeId, target: NodeId, term: u64, config: Arc<Config>, last_log_index: u64, last_log_term: u64, commit_index: u64, network: Arc<N>,
        storage: Arc<S>, replicationtx: mpsc::UnboundedSender<ReplicaEvent<S::Snapshot>>,
    ) -> Self {
        ReplicationCore::spawn(
            id,
            target,
            term,
            config,
            last_log_index,
            last_log_term,
            commit_index,
            network,
            storage,
            replicationtx,
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
    rafttx: mpsc::UnboundedSender<ReplicaEvent<S::Snapshot>>,
    /// A channel for receiving events from the Raft node.
    raftrx: mpsc::UnboundedReceiver<RaftEvent<D>>,
    /// The `RaftNetwork` interface.
    network: Arc<N>,
    /// The `RaftStorage` interface.
    storage: Arc<S>,
    /// The Raft's runtime config.
    config: Arc<Config>,
    /// The configured max payload entries, simply as a usize.
    max_payload_entries: usize,
    marker_r: std::marker::PhantomData<R>,

    //////////////////////////////////////////////////////////////////////////
    // Dynamic Fields ////////////////////////////////////////////////////////
    /// The target state of this replication stream.
    target_state: TargetReplState,

    /// The index of the log entry to most recently be appended to the log by the leader.
    last_log_index: u64,
    /// The index of the highest log entry which is known to be committed in the cluster.
    commit_index: u64,

    /// The index of the next log to send.
    ///
    /// This is initialized to leader's last log index + 1. Per the Raft protocol spec,
    /// this value may be decremented as new nodes enter the cluster and need to catch-up per the
    /// log consistency check.
    ///
    /// If a follower's log is inconsistent with the leader's, the AppendEntries consistency check
    /// will fail in the next AppendEntries RPC. After a rejection, the leader decrements
    /// `next_index` and retries the AppendEntries RPC. Eventually `next_index` will reach a point
    /// where the leader and follower logs match. When this happens, AppendEntries will succeed,
    /// which removes any conflicting entries in the follower's log and appends entries from the
    /// leader's log (if any). Once AppendEntries succeeds, the follower’s log is consistent with
    /// the leader's, and it will remain that way for the rest of the term.
    ///
    /// This Raft implementation also uses a _conflict optimization_ pattern for reducing the
    /// number of RPCs which need to be sent back and forth between a peer which is lagging
    /// behind. This is defined in §5.3.
    next_index: u64,
    /// The last know index to be successfully replicated on the target.
    ///
    /// This will be initialized to the leader's last_log_index, and will be updated as
    /// replication proceeds.
    match_index: u64,
    /// The term of the last know index to be successfully replicated on the target.
    ///
    /// This will be initialized to the leader's last_log_term, and will be updated as
    /// replication proceeds.
    match_term: u64,

    /// A buffer of data to replicate to the target follower.
    ///
    /// The buffered payload here will be expanded as more replication commands come in from the
    /// Raft node. Data from this buffer will flow into the `outbound_buffer` in chunks.
    replication_buffer: Vec<Arc<Entry<D>>>,
    /// A buffer of data which is being sent to the follower.
    ///
    /// Data in this buffer comes directly from the `replication_buffer` in chunks, and will
    /// remain here until it is confirmed that the payload has been successfully received by the
    /// target node. This allows for retransmission of payloads in the face of transient errors.
    outbound_buffer: Vec<OutboundEntry<D>>,
    /// The heartbeat interval for ensuring that heartbeats are always delivered in a timely fashion.
    heartbeat: Interval,
    /// The timeout duration for heartbeats.
    heartbeat_timeout: Duration,
}

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> ReplicationCore<D, R, N, S> {
    /// Spawn a new replication task for the target node.
    pub(self) fn spawn(
        id: NodeId, target: NodeId, term: u64, config: Arc<Config>, last_log_index: u64, last_log_term: u64, commit_index: u64, network: Arc<N>,
        storage: Arc<S>, rafttx: mpsc::UnboundedSender<ReplicaEvent<S::Snapshot>>,
    ) -> ReplicationStream<D> {
        let (raftrx_tx, raftrx) = mpsc::unbounded_channel();
        let heartbeat_timeout = Duration::from_millis(config.heartbeat_interval);
        let max_payload_entries = config.max_payload_entries as usize;
        let this = Self {
            id,
            target,
            term,
            network,
            storage,
            config,
            max_payload_entries,
            marker_r: std::marker::PhantomData,
            target_state: TargetReplState::Lagging,
            last_log_index,
            commit_index,
            next_index: last_log_index + 1,
            match_index: last_log_index,
            match_term: last_log_term,
            rafttx,
            raftrx,
            heartbeat: interval(heartbeat_timeout),
            heartbeat_timeout,
            replication_buffer: Vec::new(),
            outbound_buffer: Vec::new(),
        };
        let handle = tokio::spawn(this.main());
        ReplicationStream { handle, repltx: raftrx_tx }
    }

    #[tracing::instrument(level="trace", skip(self), fields(id=self.id, target=self.target, cluster=%self.config.cluster_name))]
    async fn main(mut self) {
        // Perform an initial heartbeat.
        self.send_append_entries().await;

        // Proceed to the replication stream's inner loop.
        loop {
            match &self.target_state {
                TargetReplState::LineRate => LineRateState::new(&mut self).run().await,
                TargetReplState::Lagging => LaggingState::new(&mut self).run().await,
                TargetReplState::Snapshotting => SnapshottingState::new(&mut self).run().await,
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
        // Attempt to fill the send buffer from the replication buffer.
        if self.outbound_buffer.is_empty() {
            let repl_len = self.replication_buffer.len();
            if repl_len > 0 {
                let chunk_size = if repl_len < self.max_payload_entries {
                    repl_len
                } else {
                    self.max_payload_entries
                };
                self.outbound_buffer
                    .extend(self.replication_buffer.drain(..chunk_size).map(OutboundEntry::Arc));
            }
        }

        // Build the heartbeat frame to be sent to the follower.
        let payload = AppendEntriesRequest {
            term: self.term,
            leader_id: self.id,
            prev_log_index: self.match_index,
            prev_log_term: self.match_term,
            leader_commit: self.commit_index,
            entries: self.outbound_buffer.iter().map(|entry| entry.as_ref().clone()).collect(),
        };

        // Send the payload.
        let res = match timeout(self.heartbeat_timeout, self.network.append_entries(self.target, payload)).await {
            Ok(outer_res) => match outer_res {
                Ok(res) => res,
                Err(err) => {
                    tracing::error!({error=%err}, "error sending AppendEntries RPC to target");
                    return;
                }
            },
            Err(err) => {
                tracing::error!({error=%err}, "timeout while sending AppendEntries RPC to target");
                return;
            }
        };
        let last_index_and_term = self.outbound_buffer.last().map(|last| (last.as_ref().index, last.as_ref().term));
        self.outbound_buffer.clear(); // Once we've successfully sent a payload of entries, don't send them again.

        // Handle success conditions.
        if res.success {
            tracing::trace!("append entries succeeded");
            // If this was a proper replication event (last index & term were provided), then update state.
            if let Some((index, term)) = last_index_and_term {
                self.next_index = index + 1; // This should always be the next expected index.
                self.match_index = index;
                self.match_term = term;
                let _ = self.rafttx.send(ReplicaEvent::UpdateMatchIndex {
                    target: self.target,
                    match_index: index,
                    match_term: term,
                });

                // If running at line rate, and our buffered outbound requests have accumulated too
                // much, we need to purge and transition to a lagging state. The target is not able to
                // replicate data fast enough.
                let is_lagging = self
                    .last_log_index
                    .checked_sub(self.match_index)
                    .map(|diff| diff > self.config.replication_lag_threshold)
                    .unwrap_or(false);
                if is_lagging {
                    self.target_state = TargetReplState::Lagging;
                }
            }
            return;
        }

        // Replication was not successful, if a newer term has been returned, revert to follower.
        if res.term > self.term {
            tracing::trace!({ res.term }, "append entries failed, reverting to follower");
            let _ = self.rafttx.send(ReplicaEvent::RevertToFollower {
                target: self.target,
                term: res.term,
            });
            self.target_state = TargetReplState::Shutdown;
            return;
        }

        // Replication was not successful, handle conflict optimization record, else decrement `next_index`.
        if let Some(conflict) = res.conflict_opt {
            tracing::trace!({?conflict, res.term}, "append entries failed, handling conflict opt");
            // If the returned conflict opt index is greater than last_log_index, then this is a
            // logical error, and no action should be taken. This represents a replication failure.
            if conflict.index > self.last_log_index {
                return;
            }
            self.next_index = conflict.index + 1;
            self.match_index = conflict.index;
            self.match_term = conflict.term;

            // If conflict index is 0, we will not be able to fetch that index from storage because
            // it will never exist. So instead, we just return, and accept the conflict data.
            if conflict.index == 0 {
                self.target_state = TargetReplState::Lagging;
                let _ = self.rafttx.send(ReplicaEvent::UpdateMatchIndex {
                    target: self.target,
                    match_index: self.match_index,
                    match_term: self.match_term,
                });
                return;
            }

            // Fetch the entry at conflict index and use the term specified there.
            match self
                .storage
                .get_log_entries(conflict.index, conflict.index + 1)
                .await
                .map(|entries| entries.get(0).map(|entry| entry.term))
            {
                Ok(Some(term)) => {
                    self.match_term = term; // If we have the specified log, ensure we use its term.
                }
                Ok(None) => {
                    // This condition would only ever be reached if the log has been removed due to
                    // log compaction (barring critical storage failure), so transition to snapshotting.
                    self.target_state = TargetReplState::Snapshotting;
                    let _ = self.rafttx.send(ReplicaEvent::UpdateMatchIndex {
                        target: self.target,
                        match_index: self.match_index,
                        match_term: self.match_term,
                    });
                    return;
                }
                Err(err) => {
                    tracing::error!({error=%err}, "error fetching log entry due to returned AppendEntries RPC conflict_opt");
                    let _ = self.rafttx.send(ReplicaEvent::Shutdown);
                    self.target_state = TargetReplState::Shutdown;
                    return;
                }
            };

            // Check snapshot policy and handle conflict as needed.
            let _ = self.rafttx.send(ReplicaEvent::UpdateMatchIndex {
                target: self.target,
                match_index: self.match_index,
                match_term: self.match_term,
            });
            match &self.config.snapshot_policy {
                SnapshotPolicy::LogsSinceLast(threshold) => {
                    let diff = self.last_log_index - conflict.index; // NOTE WELL: underflow is guarded against above.
                    if &diff >= threshold {
                        // Follower is far behind and needs to receive an InstallSnapshot RPC.
                        self.target_state = TargetReplState::Snapshotting;
                        return;
                    }
                    // Follower is behind, but not too far behind to receive an InstallSnapshot RPC.
                    self.target_state = TargetReplState::Lagging;
                    return;
                }
            }
        }
    }

    /// Perform a check to see if this replication stream is lagging behind far enough that a
    /// snapshot is warranted.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(self) fn needs_snapshot(&self) -> bool {
        match &self.config.snapshot_policy {
            SnapshotPolicy::LogsSinceLast(threshold) => {
                let needs_snap = self
                    .commit_index
                    .checked_sub(self.match_index)
                    .map(|diff| diff >= *threshold)
                    .unwrap_or(false);
                if needs_snap {
                    tracing::trace!("snapshot needed");
                    true
                } else {
                    tracing::trace!("snapshot not needed");
                    false
                }
            }
        }
    }

    /// Fully drain the channel coming in from the Raft node.
    pub(self) fn drain_raftrx(&mut self, first: RaftEvent<D>) {
        let mut event_opt = Some(first);
        let mut iters = 0;
        loop {
            // Just ensure we don't get stuck draining a REALLY hot replication feed.
            if iters > self.config.max_payload_entries {
                return;
            }
            // Unpack the event opt, else return if we don't have one to process.
            let event = match event_opt.take() {
                Some(event) => event,
                None => return,
            };
            // Process the event.
            match event {
                RaftEvent::UpdateCommitIndex { commit_index } => {
                    self.commit_index = commit_index;
                }
                RaftEvent::Replicate { entry, commit_index } => {
                    self.commit_index = commit_index;
                    self.last_log_index = entry.index;
                    if self.target_state == TargetReplState::LineRate {
                        self.replication_buffer.push(entry);
                    }
                }
                RaftEvent::Terminate => {
                    self.target_state = TargetReplState::Shutdown;
                    return;
                }
            }
            // Attempt to unpack the next event for the next loop iteration.
            if let Some(event) = self.raftrx.recv().now_or_never() {
                event_opt = event;
            }
            iters += 1;
        }
    }
}

/// A type which wraps two possible forms of an outbound entry for replication.
enum OutboundEntry<D: AppData> {
    /// An entry owned by an Arc, hot off the replication stream from the Raft leader.
    Arc(Arc<Entry<D>>),
    /// An entry which was fetched directly from storage.
    Raw(Entry<D>),
}

impl<D: AppData> AsRef<Entry<D>> for OutboundEntry<D> {
    fn as_ref(&self) -> &Entry<D> {
        match self {
            Self::Arc(inner) => inner.as_ref(),
            Self::Raw(inner) => inner,
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// The state of the replication stream.
#[derive(Eq, PartialEq)]
enum TargetReplState {
    /// The replication stream is running at line rate.
    LineRate,
    /// The replication stream is lagging behind.
    Lagging,
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
where
    S: AsyncRead + AsyncSeek + Send + Unpin + 'static,
{
    /// An event representing an update to the replication rate of a replication stream.
    RateUpdate {
        /// The ID of the Raft node to which this event relates.
        target: NodeId,
        /// A flag indicating if the corresponding target node is replicating at line rate.
        ///
        /// When replicating at line rate, the replication stream will receive log entires to
        /// replicate as soon as they are ready. When not running at line rate, the Raft node will
        /// only send over metadata without entries to replicate.
        is_line_rate: bool,
    },
    /// An event from a replication stream which updates the target node's match index.
    UpdateMatchIndex {
        /// The ID of the target node for which the match index is to be updated.
        target: NodeId,
        /// The index of the most recent log known to have been successfully replicated on the target.
        match_index: u64,
        /// The term of the most recent log known to have been successfully replicated on the target.
        match_term: u64,
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
        tx: oneshot::Sender<CurrentSnapshotData<S>>,
    },
    /// Some critical error has taken place, and Raft needs to shutdown.
    Shutdown,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// LineRate specific state.
struct LineRateState<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    /// An exclusive handle to the replication core.
    core: &'a mut ReplicationCore<D, R, N, S>,
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> LineRateState<'a, D, R, N, S> {
    /// Create a new instance.
    pub fn new(core: &'a mut ReplicationCore<D, R, N, S>) -> Self {
        Self { core }
    }

    #[tracing::instrument(level = "trace", skip(self), fields(state = "line-rate"))]
    pub async fn run(mut self) {
        let event = ReplicaEvent::RateUpdate {
            target: self.core.target,
            is_line_rate: true,
        };
        let _ = self.core.rafttx.send(event);
        loop {
            if self.core.target_state != TargetReplState::LineRate {
                return;
            }

            // We always prioritize draining our buffers first.
            let next_buf_index = self
                .core
                .outbound_buffer
                .first()
                .map(|entry| entry.as_ref().index)
                .or_else(|| self.core.replication_buffer.first().map(|entry| entry.index));
            if let Some(index) = next_buf_index {
                // Ensure that our buffered data matches up with `next_index`. When transitioning to
                // line rate, it is always possible that new data has been sent for replication but has
                // skipped this replication stream during transition. In such cases, a single update from
                // storage will put this stream back on track.
                if self.core.next_index != index {
                    self.frontload_outbound_buffer(self.core.next_index, index).await;
                    if self.core.target_state != TargetReplState::LineRate {
                        return;
                    }
                }

                self.core.send_append_entries().await;
                continue;
            }
            tokio::select! {
                _ = self.core.heartbeat.tick() => self.core.send_append_entries().await,
                event = self.core.raftrx.recv() => match event {
                    Some(event) => self.core.drain_raftrx(event),
                    None => self.core.target_state = TargetReplState::Shutdown,
                }
            }
        }
    }

    /// Ensure there are no gaps in the outbound buffer due to transition from lagging.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn frontload_outbound_buffer(&mut self, start: u64, stop: u64) {
        let entries = match self.core.storage.get_log_entries(start, stop).await {
            Ok(entries) => entries,
            Err(err) => {
                tracing::error!({error=%err}, "error while frontloading outbound buffer");
                let _ = self.core.rafttx.send(ReplicaEvent::Shutdown);
                return;
            }
        };
        for entry in entries.iter() {
            if let EntryPayload::SnapshotPointer(_) = entry.payload {
                self.core.target_state = TargetReplState::Snapshotting;
                return;
            }
        }
        // Prepend.
        self.core.outbound_buffer.reverse();
        self.core.outbound_buffer.extend(entries.into_iter().rev().map(OutboundEntry::Raw));
        self.core.outbound_buffer.reverse();
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// Lagging specific state.
struct LaggingState<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    /// An exclusive handle to the replication core.
    core: &'a mut ReplicationCore<D, R, N, S>,
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> LaggingState<'a, D, R, N, S> {
    /// Create a new instance.
    pub fn new(core: &'a mut ReplicationCore<D, R, N, S>) -> Self {
        Self { core }
    }

    #[tracing::instrument(level = "trace", skip(self), fields(state = "lagging"))]
    pub async fn run(mut self) {
        let event = ReplicaEvent::RateUpdate {
            target: self.core.target,
            is_line_rate: false,
        };
        let _ = self.core.rafttx.send(event);
        self.core.replication_buffer.clear();
        self.core.outbound_buffer.clear();
        loop {
            if self.core.target_state != TargetReplState::Lagging {
                return;
            }
            // If this stream is far enough behind, then transition to snapshotting state.
            if self.core.needs_snapshot() {
                self.core.target_state = TargetReplState::Snapshotting;
                return;
            }

            // Prep entries from storage and send them off for replication.
            if self.is_up_to_speed() {
                self.core.target_state = TargetReplState::LineRate;
                return;
            }
            self.prep_outbound_buffer_from_storage().await;
            self.core.send_append_entries().await;
            if self.is_up_to_speed() {
                self.core.target_state = TargetReplState::LineRate;
                return;
            }

            // Check raft channel to ensure we are staying up-to-date, then loop.
            if let Some(Some(event)) = self.core.raftrx.recv().now_or_never() {
                self.core.drain_raftrx(event);
            }
        }
    }

    /// Check if this replication stream is now up-to-speed.
    #[tracing::instrument(level="trace", skip(self), fields(self.core.next_index, self.core.commit_index))]
    fn is_up_to_speed(&self) -> bool {
        self.core.next_index > self.core.commit_index
    }

    /// Prep the outbound buffer with the next payload of entries to append.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn prep_outbound_buffer_from_storage(&mut self) {
        // If the send buffer is empty, we need to fill it.
        if self.core.outbound_buffer.is_empty() {
            // Determine an appropriate stop index for the storage fetch operation. Avoid underflow.
            let distance_behind = self.core.commit_index - self.core.next_index; // Underflow is guarded against in the `is_up_to_speed` check in the outer loop.
            let is_within_payload_distance = distance_behind <= self.core.config.max_payload_entries;
            let stop_idx = if is_within_payload_distance {
                // If we have caught up to the line index, then that means we will be running at
                // line rate after this payload is successfully replicated.
                self.core.target_state = TargetReplState::LineRate; // Will continue in lagging state until the outer loop cycles.
                self.core.commit_index + 1 // +1 to ensure stop value is included.
            } else {
                self.core.next_index + self.core.config.max_payload_entries + 1 // +1 to ensure stop value is included.
            };

            // Bringing the target up-to-date by fetching the largest possible payload of entries
            // from storage within permitted configuration & ensure no snapshot pointer was returned.
            let entries = match self.core.storage.get_log_entries(self.core.next_index, stop_idx).await {
                Ok(entries) => entries,
                Err(err) => {
                    tracing::error!({error=%err}, "error fetching logs from storage");
                    let _ = self.core.rafttx.send(ReplicaEvent::Shutdown);
                    self.core.target_state = TargetReplState::Shutdown;
                    return;
                }
            };
            for entry in entries.iter() {
                if let EntryPayload::SnapshotPointer(_) = entry.payload {
                    self.core.target_state = TargetReplState::Snapshotting;
                    return;
                }
            }
            self.core.outbound_buffer.extend(entries.into_iter().map(OutboundEntry::Raw));
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// Snapshotting specific state.
struct SnapshottingState<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    /// An exclusive handle to the replication core.
    core: &'a mut ReplicationCore<D, R, N, S>,
    snapshot: Option<CurrentSnapshotData<S::Snapshot>>,
    snapshot_fetch_rx: Option<oneshot::Receiver<CurrentSnapshotData<S::Snapshot>>>,
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> SnapshottingState<'a, D, R, N, S> {
    /// Create a new instance.
    pub fn new(core: &'a mut ReplicationCore<D, R, N, S>) -> Self {
        Self {
            core,
            snapshot: None,
            snapshot_fetch_rx: None,
        }
    }

    #[tracing::instrument(level = "trace", skip(self), fields(state = "snapshotting"))]
    pub async fn run(mut self) {
        let event = ReplicaEvent::RateUpdate {
            target: self.core.target,
            is_line_rate: false,
        };
        let _ = self.core.rafttx.send(event);
        self.core.replication_buffer.clear();
        self.core.outbound_buffer.clear();

        loop {
            if self.core.target_state != TargetReplState::Snapshotting {
                return;
            }

            // If we don't have any of the components we need, fetch the current snapshot.
            if self.snapshot.is_none() && self.snapshot_fetch_rx.is_none() {
                let (tx, rx) = oneshot::channel();
                let _ = self.core.rafttx.send(ReplicaEvent::NeedsSnapshot {
                    target: self.core.target,
                    tx,
                });
                self.snapshot_fetch_rx = Some(rx);
            }

            // If we are waiting for a snapshot response from the storage layer, then wait for
            // it and send heartbeats in the meantime.
            if let Some(snapshot_fetch_rx) = self.snapshot_fetch_rx.take() {
                self.wait_for_snapshot(snapshot_fetch_rx).await;
                continue;
            }

            // If we have a snapshot to work with, then stream it.
            if let Some(snapshot) = self.snapshot.take() {
                if let Err(err) = self.stream_snapshot(snapshot).await {
                    tracing::error!({error=%err}, "error streaming snapshot to target");
                }
                continue;
            }
        }
    }

    /// Wait for a response from the storage layer for the current snapshot.
    ///
    /// If an error comes up during processing, this routine should simple be called again after
    /// issuing a new request to the storage layer.
    #[tracing::instrument(level = "trace", skip(self, rx))]
    async fn wait_for_snapshot(&mut self, mut rx: oneshot::Receiver<CurrentSnapshotData<S::Snapshot>>) {
        loop {
            tokio::select! {
                _ = self.core.heartbeat.tick() => self.core.send_append_entries().await,
                event = self.core.raftrx.recv() => match event {
                    Some(event) => self.core.drain_raftrx(event),
                    None => {
                        self.core.target_state = TargetReplState::Shutdown;
                        return;
                    }
                },
                res = &mut rx => {
                    match res {
                        Ok(snapshot) => {
                            self.snapshot = Some(snapshot);
                            return;
                        }
                        Err(_) => return, // Channels may close for various acceptable reasons.
                    }
                },
            }
        }
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn stream_snapshot(&mut self, mut snapshot: CurrentSnapshotData<S::Snapshot>) -> RaftResult<()> {
        let mut offset = 0;
        self.core.next_index = snapshot.index + 1;
        self.core.match_index = snapshot.index;
        self.core.match_term = snapshot.term;
        let mut buf = Vec::with_capacity(self.core.config.snapshot_max_chunk_size as usize);
        loop {
            // Build the RPC.
            snapshot.snapshot.seek(SeekFrom::Start(offset)).await?;
            let nread = snapshot.snapshot.read_buf(&mut buf).await?;
            let done = nread == 0; // If bytes read == 0, then we're done.
            let req = InstallSnapshotRequest {
                term: self.core.term,
                leader_id: self.core.id,
                last_included_index: snapshot.index,
                last_included_term: snapshot.term,
                offset,
                data: Vec::from(&buf[..nread]),
                done,
            };
            buf.clear();

            // Send the RPC over to the target.
            tracing::trace!({snapshot_size=req.data.len(), nread, req.done, req.offset}, "sending snapshot chunk");
            let res = match timeout(self.core.heartbeat_timeout, self.core.network.install_snapshot(self.core.target, req)).await {
                Ok(outer_res) => match outer_res {
                    Ok(res) => res,
                    Err(err) => {
                        tracing::error!({error=%err}, "error sending InstallSnapshot RPC to target");
                        continue;
                    }
                },
                Err(err) => {
                    tracing::error!({error=%err}, "timeout while sending InstallSnapshot RPC to target");
                    continue;
                }
            };

            // Handle response conditions.
            if res.term > self.core.term {
                let _ = self.core.rafttx.send(ReplicaEvent::RevertToFollower {
                    target: self.core.target,
                    term: res.term,
                });
                self.core.target_state = TargetReplState::Shutdown;
                return Ok(());
            }

            // If we just sent the final chunk of the snapshot, then transition to lagging state.
            if done {
                self.core.target_state = TargetReplState::Lagging;
                return Ok(());
            }

            // Everything is good, so update offset for sending the next chunk.
            offset += nread as u64;

            // Check raft channel to ensure we are staying up-to-date, then loop.
            if let Some(Some(event)) = self.core.raftrx.recv().now_or_never() {
                self.core.drain_raftrx(event);
            }
        }
    }
}
