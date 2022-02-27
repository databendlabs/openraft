//! Replication stream.

use std::io::SeekFrom;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
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
use crate::error::AppendEntriesError;
use crate::error::CommittedAdvanceTooMany;
use crate::error::HigherVote;
use crate::error::LackEntry;
use crate::error::RPCError;
use crate::error::ReplicationError;
use crate::error::Timeout;
use crate::raft::AppendEntriesRequest;
use crate::raft::InstallSnapshotRequest;
use crate::raft_types::LogIdOptionExt;
use crate::raft_types::LogIndexOptionExt;
use crate::storage::RaftLogReader;
use crate::storage::Snapshot;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::LeaderId;
use crate::LogId;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RPCTypes;
use crate::RaftNetwork;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::ToStorageResult;
use crate::Vote;

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ReplicationMetrics {
    pub(crate) matched_leader_id: Option<LeaderId>,
    pub(crate) matched_index: AtomicU64,
}

impl Clone for ReplicationMetrics {
    fn clone(&self) -> Self {
        Self {
            matched_leader_id: self.matched_leader_id,
            matched_index: AtomicU64::new(self.matched_index.load(Ordering::Relaxed)),
        }
    }
}

impl PartialEq for ReplicationMetrics {
    fn eq(&self, other: &Self) -> bool {
        self.matched_leader_id == other.matched_leader_id
            && self.matched_index.load(Ordering::Relaxed) == other.matched_index.load(Ordering::Relaxed)
    }
}

impl Eq for ReplicationMetrics {}

impl ReplicationMetrics {
    pub fn new(log_id: Option<LogId>) -> Self {
        if let Some(log_id) = log_id {
            Self {
                matched_leader_id: Some(log_id.leader_id),
                matched_index: AtomicU64::new(log_id.index),
            }
        } else {
            Self::default()
        }
    }
    pub fn matched(&self) -> Option<LogId> {
        if let Some(leader_id) = self.matched_leader_id {
            let index = self.matched_index.load(Ordering::Relaxed);
            Some(LogId { leader_id, index })
        } else {
            None
        }
    }
}

impl MessageSummary for ReplicationMetrics {
    fn summary(&self) -> String {
        format!("{:?}", self.matched())
    }
}

/// The public handle to a spawned replication stream.
pub(crate) struct ReplicationStream {
    /// The spawn handle the `ReplicationCore` task.
    // pub handle: JoinHandle<()>,
    /// The channel used for communicating with the replication task.
    pub repl_tx: mpsc::UnboundedSender<(RaftEvent, Span)>,
}

impl ReplicationStream {
    /// Create a new replication stream for the target peer.
    pub(crate) fn new<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>>(
        target: NodeId,
        target_node: Option<Node>,
        vote: Vote,
        config: Arc<Config>,
        last_log: Option<LogId>,
        committed: Option<LogId>,
        network: N::Network,
        log_reader: S::LogReader,
        replication_tx: mpsc::UnboundedSender<(ReplicaEvent<S::SnapshotData>, Span)>,
    ) -> Self {
        ReplicationCore::<C, N, S>::spawn(
            target,
            target_node,
            vote,
            config,
            last_log,
            committed,
            network,
            log_reader,
            replication_tx,
        )
    }
}

/// A task responsible for sending replication events to a target follower in the Raft cluster.
///
/// NOTE: we do not stack replication requests to targets because this could result in
/// out-of-order delivery. We always buffer until we receive a success response, then send the
/// next payload from the buffer.
struct ReplicationCore<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    /// The ID of the target Raft node which replication events are to be sent to.
    target: NodeId,

    /// The vote of the leader.
    vote: Vote,

    /// A channel for sending events to the Raft node.
    raft_core_tx: mpsc::UnboundedSender<(ReplicaEvent<S::SnapshotData>, Span)>,

    /// A channel for receiving events from the Raft node.
    repl_rx: mpsc::UnboundedReceiver<(RaftEvent, Span)>,

    /// The `RaftNetwork` interface.
    network: N::Network,

    /// The `RaftLogReader` of a `RaftStorage` interface.
    log_reader: S::LogReader,

    /// The Raft's runtime config.
    config: Arc<Config>,

    //////////////////////////////////////////////////////////////////////////
    // Dynamic Fields ////////////////////////////////////////////////////////
    /// The target state of this replication stream.
    target_repl_state: TargetReplState,

    /// The id of the log entry to most recently be appended to the log by the leader.
    last_log_id: Option<LogId>,

    /// The log id of the highest log entry which is known to be committed in the cluster.
    committed: Option<LogId>,

    /// The last know log to be successfully replicated on the target.
    ///
    /// This Raft implementation also uses a _conflict optimization_ pattern for reducing the
    /// number of RPCs which need to be sent back and forth between a peer which is lagging
    /// behind. This is defined in §5.3.
    /// This will be initialized to the leader's (last_log_term, last_log_index), and will be updated as
    /// replication proceeds.
    matched: Option<LogId>,

    // The last possible matching entry on a follower.
    max_possible_matched_index: Option<u64>,

    /// The heartbeat interval for ensuring that heartbeats are always delivered in a timely fashion.
    heartbeat: Interval,

    /// The timeout for sending snapshot segment.
    install_snapshot_timeout: Duration,
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> ReplicationCore<C, N, S> {
    /// Spawn a new replication task for the target node.
    #[tracing::instrument(level = "trace", skip(config, network, log_reader, raft_core_tx))]
    pub(self) fn spawn(
        target: NodeId,
        target_node: Option<Node>,
        vote: Vote,
        config: Arc<Config>,
        last_log: Option<LogId>,
        committed: Option<LogId>,
        network: N::Network,
        log_reader: S::LogReader,
        raft_core_tx: mpsc::UnboundedSender<(ReplicaEvent<S::SnapshotData>, Span)>,
    ) -> ReplicationStream {
        // other component to ReplicationStream
        let (repl_tx, repl_rx) = mpsc::unbounded_channel();
        let heartbeat_timeout = Duration::from_millis(config.heartbeat_interval);
        let install_snapshot_timeout = Duration::from_millis(config.install_snapshot_timeout);

        let this = Self {
            target,
            vote,
            network,
            log_reader,
            config,
            target_repl_state: TargetReplState::LineRate,
            last_log_id: last_log,
            committed,
            matched: None,
            max_possible_matched_index: last_log.index(),
            raft_core_tx,
            repl_rx,
            heartbeat: interval(heartbeat_timeout),
            install_snapshot_timeout,
        };

        let _handle = tokio::spawn(this.main().instrument(tracing::trace_span!("spawn").or_current()));

        ReplicationStream {
            // handle,
            repl_tx,
        }
    }

    #[tracing::instrument(level="trace", skip(self), fields(vote=%self.vote, target=display(self.target), cluster=%self.config.cluster_name))]
    async fn main(mut self) {
        loop {
            // If it returns Ok(), always go back to LineRate state.
            let res = match &self.target_repl_state {
                TargetReplState::LineRate => self.line_rate_loop().await,
                TargetReplState::Snapshotting { must_include } => {
                    let must = *must_include;
                    self.replicate_snapshot(must).await
                }
                TargetReplState::Shutdown => return,
            };

            let err = match res {
                Ok(_) => {
                    self.set_target_repl_state(TargetReplState::LineRate);
                    continue;
                }
                Err(err) => err,
            };

            tracing::warn!(error=%err, "error replication to target={}", self.target);

            match err {
                ReplicationError::Closed => {
                    self.set_target_repl_state(TargetReplState::Shutdown);
                }
                ReplicationError::HigherVote(h) => {
                    let _ = self.raft_core_tx.send((
                        ReplicaEvent::RevertToFollower {
                            target: self.target,
                            vote: h.higher,
                        },
                        tracing::debug_span!("CH"),
                    ));
                    return;
                }
                ReplicationError::LackEntry(lack_ent) => {
                    self.set_target_repl_state(TargetReplState::Snapshotting {
                        must_include: lack_ent.last_purged_log_id,
                    });
                }
                ReplicationError::CommittedAdvanceTooMany { .. } => {
                    self.set_target_repl_state(TargetReplState::Snapshotting { must_include: None });
                }
                ReplicationError::StorageError(_err) => {
                    self.set_target_repl_state(TargetReplState::Shutdown);
                    let _ = self.raft_core_tx.send((ReplicaEvent::Shutdown, tracing::debug_span!("CH")));
                    return;
                }
                ReplicationError::NodeNotFound(err) => {
                    unreachable!("programming bug: {}", err)
                }
                ReplicationError::Timeout { .. } => {
                    // nothing to do
                }
                ReplicationError::Network { .. } => {
                    // nothing to do
                }
                ReplicationError::RemoteError(remote_err) => {
                    tracing::error!(%remote_err, "remote peer error");
                    match remote_err.source {
                        AppendEntriesError::Fatal(fatal) => {
                            tracing::error!(%fatal, target=%remote_err.target, "remote fatal error, close replication");
                            return;
                        }
                    }
                }
            };
        }
    }

    /// Send an AppendEntries RPC to the target.
    ///
    /// This request will timeout if no response is received within the
    /// configured heartbeat interval.
    #[tracing::instrument(level = "debug", skip(self))]
    async fn send_append_entries(&mut self) -> Result<(), ReplicationError> {
        // find the mid position aligning to 8
        let diff = self.max_possible_matched_index.next_index() - self.matched.next_index();
        let offset = diff / 16 * 8;

        let mut prev_index = self.matched.index().add(offset);

        let (prev_log_id, logs) = loop {
            // TODO(xp): test heartbeat when all logs are removed.

            let log_state = self.log_reader.get_log_state().await?;

            let last_purged = log_state.last_purged_log_id;

            self.check_consecutive(last_purged)?;

            if prev_index < last_purged.index() {
                prev_index = last_purged.index();
            }

            let start = prev_index.next_index();
            let end = std::cmp::min(
                start + self.config.max_payload_entries,
                log_state.last_log_id.next_index(),
            );

            tracing::debug!(
                ?self.matched,
                ?self.max_possible_matched_index,
                ?last_purged,
                ?prev_index,
                end,
                "load entries",
            );

            assert!(end >= prev_index.next_index());

            let prev_log_id = if prev_index == last_purged.index() {
                last_purged
            } else if let Some(prev_i) = prev_index {
                let first = self.log_reader.try_get_log_entry(prev_i).await?;
                match first {
                    Some(f) => Some(f.log_id),
                    None => {
                        tracing::info!("can not load first entry: at {:?}, retry loading logs", prev_index);
                        continue;
                    }
                }
            } else {
                None
            };

            let logs = if start == end {
                vec![]
            } else {
                let logs = self.log_reader.try_get_log_entries(start..end).await?;
                if !logs.is_empty() && logs[0].log_id.index > prev_log_id.next_index() {
                    // There is still chance the first log is removed.
                    // log entry is just deleted after fetching first_log_id.
                    // Without consecutive logs, we have to retry loading.
                    continue;
                }

                logs
            };

            break (prev_log_id, logs);
        };

        let conflict = prev_log_id;
        let matched = if logs.is_empty() {
            prev_log_id
        } else {
            Some(logs[logs.len() - 1].log_id)
        };

        // Build the heartbeat frame to be sent to the follower.
        let payload = AppendEntriesRequest {
            vote: self.vote,
            prev_log_id,
            leader_commit: self.committed,
            entries: logs,
        };

        // Send the payload.
        tracing::debug!(
            payload=%payload.summary(),
            "start sending append_entries, timeout: {:?}",
            self.config.heartbeat_interval
        );

        let the_timeout = Duration::from_millis(self.config.heartbeat_interval);
        let res = timeout(the_timeout, self.network.send_append_entries(payload)).await;

        let append_resp = match res {
            Ok(append_res) => match append_res {
                Ok(res) => res,
                Err(err) => {
                    tracing::warn!(error=%err, "error sending AppendEntries RPC to target");
                    let repl_err = match err {
                        RPCError::NodeNotFound(e) => ReplicationError::NodeNotFound(e),
                        RPCError::Timeout(e) => ReplicationError::Timeout(e),
                        RPCError::Network(e) => ReplicationError::Network(e),
                        RPCError::RemoteError(e) => ReplicationError::RemoteError(e),
                    };
                    return Err(repl_err);
                }
            },
            Err(timeout_err) => {
                tracing::warn!(error=%timeout_err, "timeout while sending AppendEntries RPC to target");
                return Err(ReplicationError::Timeout(Timeout {
                    action: RPCTypes::AppendEntries,
                    id: self.vote.node_id,
                    target: self.target,
                    timeout: the_timeout,
                }));
            }
        };

        tracing::debug!("append_entries resp: {:?}", append_resp);

        // Handle success conditions.
        if append_resp.success {
            self.update_matched(matched);
            return Ok(());
        }

        // Failed

        // Replication was not successful, if a newer term has been returneCevert to follower.
        if append_resp.vote > self.vote {
            tracing::debug!(%append_resp.vote, "append entries faileCeverting to follower");

            return Err(ReplicationError::HigherVote(HigherVote {
                higher: append_resp.vote,
                mine: self.vote,
            }));
        }

        tracing::debug!(
            ?conflict,
            %append_resp.vote,
            "append entries failed, handling conflict opt"
        );

        assert!(conflict.is_some(), "prev_log_id=None never conflict");
        let conflict = conflict.unwrap();

        // Continue to find the matching log id on follower.
        self.max_possible_matched_index = if conflict.index == 0 {
            None
        } else {
            Some(conflict.index - 1)
        };

        Ok(())
    }

    /// max_possible_matched_index is the least index for `prev_log_id` to form a consecutive log sequence
    #[tracing::instrument(level = "trace", skip(self), fields(max_possible_matched_index=self.max_possible_matched_index))]
    fn check_consecutive(&self, last_purged: Option<LogId>) -> Result<(), ReplicationError> {
        tracing::debug!(?last_purged, ?self.max_possible_matched_index, "check_consecutive");

        if last_purged.index() > self.max_possible_matched_index {
            return Err(ReplicationError::LackEntry(LackEntry {
                index: self.max_possible_matched_index,
                last_purged_log_id: last_purged,
            }));
        }

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn set_target_repl_state(&mut self, state: TargetReplState) {
        tracing::debug!(?state, "set_target_repl_state");
        self.target_repl_state = state;
    }

    /// Update the `matched` and `max_possible_matched_index`, which both are for tracking
    /// follower replication(the left and right cursor in a bsearch).
    /// And also report the matched log id to RaftCore to commit an entry etc.
    #[tracing::instrument(level = "trace", skip(self))]
    fn update_matched(&mut self, new_matched: Option<LogId>) {
        tracing::debug!(
            self.max_possible_matched_index,
            ?self.matched,
            ?new_matched, "update_matched");

        if self.max_possible_matched_index < new_matched.index() {
            self.max_possible_matched_index = new_matched.index();
        }

        if self.matched < new_matched {
            self.matched = new_matched;

            tracing::debug!(target=%self.target, matched=?self.matched, "matched updated");

            let _ = self.raft_core_tx.send((
                ReplicaEvent::UpdateMatched {
                    target: self.target,
                    matched: self.matched,
                },
                tracing::debug_span!("CH"),
            ));
        }
    }

    /// Perform a check to see if this replication stream is lagging behind far enough that a
    /// snapshot is warranted.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(self) fn needs_snapshot(&self) -> bool {
        match &self.config.snapshot_policy {
            SnapshotPolicy::LogsSinceLast(threshold) => {
                let c = self.committed.next_index();
                let m = self.matched.next_index();

                let needs_snap = c.saturating_sub(m) >= *threshold;

                tracing::trace!("snapshot needed: {}", needs_snap);
                needs_snap
            }
        }
    }

    /// Perform a check to see if this replication stream has more log to replicate
    #[tracing::instrument(level = "trace", skip(self))]
    pub(self) fn has_more_log(&self) -> bool {
        self.last_log_id.index() > self.matched.index()
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn try_drain_raft_rx(&mut self) -> Result<(), ReplicationError> {
        tracing::debug!("try_drain_raft_rx");

        for _i in 0..self.config.max_payload_entries {
            let ev = self.repl_rx.recv().now_or_never();
            let ev = match ev {
                None => {
                    // no event in self.repl_rx
                    return Ok(());
                }
                Some(x) => x,
            };

            let ev_and_span = match ev {
                None => {
                    // channel is closed, Leader quited.
                    return Err(ReplicationError::Closed);
                }
                Some(x) => x,
            };

            // TODO(xp): the span is not used. remove it from event.
            self.process_raft_event(ev_and_span.0)?;
        }

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self), fields(event=%event.summary()))]
    pub fn process_raft_event(&mut self, event: RaftEvent) -> Result<(), ReplicationError> {
        tracing::debug!(event=%event.summary(), "process_raft_event");

        match event {
            RaftEvent::UpdateCommittedLogId { committed } => {
                self.committed = committed;
            }

            RaftEvent::Replicate { appended, committed } => {
                self.committed = committed;

                assert!(self.last_log_id < Some(appended));
                self.last_log_id = Some(appended);
            }
        }

        Ok(())
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////

/// The state of the replication stream.
#[derive(Debug, Eq, PartialEq)]
enum TargetReplState {
    /// The replication stream is running at line rate.
    LineRate,

    /// The replication stream is streaming a snapshot over to the target node.
    Snapshotting { must_include: Option<LogId> },

    /// The replication stream is shutting down.
    Shutdown,
}

// TODO(xp): remove Replicate
/// An event from the Raft node.
pub(crate) enum RaftEvent {
    Replicate {
        /// The new entry which needs to be replicated.
        ///
        /// The logId of the most recent entry to have been appended to the log, its index is the
        /// new last_log_index value.
        appended: LogId,

        /// The index of the highest log entry which is known to be committed in the cluster.
        committed: Option<LogId>,
    },
    /// A message from Raft indicating a new commit index value.
    UpdateCommittedLogId {
        /// The index of the highest log entry which is known to be committed in the cluster.
        committed: Option<LogId>,
    },
}

impl MessageSummary for RaftEvent {
    fn summary(&self) -> String {
        match self {
            RaftEvent::Replicate { appended: _, committed } => {
                format!("Replicate: committed: {:?}", committed)
            }
            RaftEvent::UpdateCommittedLogId {
                committed: commit_index,
            } => {
                format!("UpdateCommitIndex: commit_index: {:?}", commit_index)
            }
        }
    }
}

/// An event coming from a replication stream.
pub(crate) enum ReplicaEvent<S>
where S: AsyncRead + AsyncSeek + Send + Unpin + 'static
{
    /// An event from a replication stream which updates the target node's match index.
    UpdateMatched {
        /// The ID of the target node for which the match index is to be updated.
        target: NodeId,
        /// The log of the most recent log known to have been successfully replicated on the target.
        matched: Option<LogId>,
    },
    /// An event indicating that the Raft node needs to revert to follower state.
    RevertToFollower {
        /// The ID of the target node from which the new term was observed.
        target: NodeId,

        /// The new vote observed.
        vote: Vote,
    },
    /// An event from a replication stream requesting snapshot info.
    NeedsSnapshot {
        target: NodeId,

        /// The log id the caller requires the snapshot has to include.
        must_include: Option<LogId>,

        /// The response channel for delivering the snapshot data.
        tx: oneshot::Sender<Snapshot<S>>,
    },
    /// Some critical error has taken place, and Raft needs to shutdown.
    Shutdown,
}

impl<S: AsyncRead + AsyncSeek + Send + Unpin + 'static> MessageSummary for ReplicaEvent<S> {
    fn summary(&self) -> String {
        match self {
            ReplicaEvent::UpdateMatched {
                ref target,
                ref matched,
            } => {
                format!("UpdateMatchIndex: target: {}, matched: {:?}", target, matched)
            }
            ReplicaEvent::RevertToFollower { ref target, ref vote } => {
                format!("RevertToFollower: target: {}, vote: {}", target, vote)
            }
            ReplicaEvent::NeedsSnapshot {
                ref target,
                ref must_include,
                ..
            } => {
                format!("NeedsSnapshot: target: {}, must_include: {:?}", target, must_include)
            }
            ReplicaEvent::Shutdown => "Shutdown".to_string(),
        }
    }
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> ReplicationCore<C, N, S> {
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn line_rate_loop(&mut self) -> Result<(), ReplicationError> {
        loop {
            loop {
                tracing::debug!(
                    "current matched: {:?} max_possible_matched_index: {:?}",
                    self.matched,
                    self.max_possible_matched_index
                );

                let res = self.send_append_entries().await;

                if let Err(err) = res {
                    tracing::error!(error=%err, "error replication to target={}", self.target);

                    // For transport error, just keep retrying.
                    match err {
                        ReplicationError::Timeout { .. } => {
                            break;
                        }
                        ReplicationError::Network { .. } => {
                            break;
                        }
                        _ => {
                            return Err(err);
                        }
                    }
                }

                if self.matched.index() == self.max_possible_matched_index {
                    break;
                }
            }

            if self.needs_snapshot() {
                return Err(ReplicationError::CommittedAdvanceTooMany(CommittedAdvanceTooMany {
                    // TODO(xp) fill them
                    committed_index: 0,
                    target_index: 0,
                }));
            }

            let span = tracing::debug_span!("CHrx:LineRate");
            let _en = span.enter();

            // Check raft channel to ensure we are staying up-to-date
            self.try_drain_raft_rx().await?;
            if self.has_more_log() {
                // if there is more log, continue to send_append_entries
                continue;
            }

            tokio::select! {
                _ = self.heartbeat.tick() => {
                    tracing::debug!("heartbeat triggered");
                    // continue
                }

                event_span = self.repl_rx.recv() => {
                    match event_span {
                        Some((event, _span)) => {
                            self.process_raft_event(event)?;
                            self.try_drain_raft_rx().await?;
                        },
                        None => {
                            tracing::debug!("received: RaftEvent::Terminate: closed");
                            return Err(ReplicationError::Closed);
                        },
                    }
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip(self), fields(state = "snapshotting"))]
    pub async fn replicate_snapshot(&mut self, snapshot_must_include: Option<LogId>) -> Result<(), ReplicationError> {
        let snapshot = self.wait_for_snapshot(snapshot_must_include).await?;
        self.stream_snapshot(snapshot).await?;

        Ok(())
    }

    /// Wait for a response from the storage layer for the current snapshot.
    ///
    /// If an error comes up during processing, this routine should simple be called again after
    /// issuing a new request to the storage layer.
    #[tracing::instrument(level = "debug", skip(self))]
    async fn wait_for_snapshot(
        &mut self,
        snapshot_must_include: Option<LogId>,
    ) -> Result<Snapshot<S::SnapshotData>, ReplicationError> {
        // Ask raft core for a snapshot.
        // - If raft core has a ready snapshot, it sends back through tx.
        // - Otherwise raft core starts a new task taking snapshot, and **close** `tx` when finished. Thus there has to
        //   be a loop.

        loop {
            // channel to communicate with raft-core
            let (tx, mut rx) = oneshot::channel();

            // TODO(xp): handle sending error. If channel is closed, quite replication by returning
            // ReplicationError::Closed.
            let _ = self.raft_core_tx.send((
                ReplicaEvent::NeedsSnapshot {
                    target: self.target,
                    must_include: snapshot_must_include,
                    tx,
                },
                tracing::debug_span!("CH"),
            ));

            let mut waiting_for_snapshot = true;

            // TODO(xp): use a watch channel to let the core to send one of the 3 event:
            //           heartbeat, new-log, or snapshot is ready.
            while waiting_for_snapshot {
                tokio::select! {
                    _ = self.heartbeat.tick() => {
                        // TODO(xp): just heartbeat:
                        let res = self.send_append_entries().await;
                        match res {
                            Ok(_) => {
                                //
                            },
                            Err(err) => {
                                if let ReplicationError::StorageError(_) = err {
                                    return Err(err);
                                }
                            }
                        }
                    },

                    event_span = self.repl_rx.recv() =>  {
                        match event_span {

                            Some((event, _span)) => {
                                self.process_raft_event(event)?;
                                self.try_drain_raft_rx().await?
                            },
                            None => {
                                tracing::info!("repl_rx is closed");
                                return Err(ReplicationError::Closed);
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
                                //           Start a new round to get the snapshot.

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
    async fn stream_snapshot(&mut self, mut snapshot: Snapshot<S::SnapshotData>) -> Result<(), ReplicationError> {
        let err_x = || (ErrorSubject::Snapshot(snapshot.meta.clone()), ErrorVerb::Read);

        let end = snapshot.snapshot.seek(SeekFrom::End(0)).await.sto_res(err_x)?;

        let mut offset = 0;

        let mut buf = Vec::with_capacity(self.config.snapshot_max_chunk_size as usize);

        loop {
            // Build the RPC.
            snapshot.snapshot.seek(SeekFrom::Start(offset)).await.sto_res(err_x)?;

            let n_read = snapshot.snapshot.read_buf(&mut buf).await.sto_res(err_x)?;

            let done = (offset + n_read as u64) == end; // If bytes read == 0, then we're done.
            let req = InstallSnapshotRequest {
                vote: self.vote,
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

            let res = timeout(self.install_snapshot_timeout, self.network.send_install_snapshot(req)).await;

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
            if res.vote > self.vote {
                return Err(ReplicationError::HigherVote(HigherVote {
                    higher: res.vote,
                    mine: self.vote,
                }));
            }

            // If we just sent the final chunk of the snapshot, then transition to lagging state.
            if done {
                tracing::debug!(
                    "done install snapshot: snapshot last_log_id: {}, matched: {:?}",
                    snapshot.meta.last_log_id,
                    self.matched,
                );

                self.update_matched(Some(snapshot.meta.last_log_id));

                return Ok(());
            }

            // Everything is good, so update offset for sending the next chunk.
            offset += n_read as u64;

            // Check raft channel to ensure we are staying up-to-date, then loop.
            self.try_drain_raft_rx().await?;
        }
    }
}
