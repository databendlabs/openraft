//! Replication stream.

mod replication_session_id;
use std::io::SeekFrom;
use std::sync::Arc;

use futures::future::FutureExt;
pub(crate) use replication_session_id::ReplicationSessionId;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio::time::timeout;
use tokio::time::Duration;
use tracing_futures::Instrument;

use crate::config::Config;
use crate::error::CommittedAdvanceTooMany;
use crate::error::HigherVote;
use crate::error::LackEntry;
use crate::error::RPCError;
use crate::error::ReplicationError;
use crate::error::Timeout;
use crate::progress::entry::ProgressEntry;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::InstallSnapshotRequest;
use crate::raft::RaftMsg;
use crate::raft_types::LogIdOptionExt;
use crate::raft_types::LogIndexOptionExt;
use crate::storage::RaftLogReader;
use crate::storage::Snapshot;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
use crate::RPCTypes;
use crate::RaftNetwork;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::SnapshotPolicy;
use crate::ToStorageResult;

/// The handle to a spawned replication stream.
pub(crate) struct ReplicationHandle<NID: NodeId> {
    /// The spawn handle the `ReplicationCore` task.
    pub(crate) join_handle: JoinHandle<()>,

    /// The channel used for communicating with the replication task.
    pub(crate) tx_repl: mpsc::UnboundedSender<Replicate<NID>>,
}

/// A task responsible for sending replication events to a target follower in the Raft cluster.
///
/// NOTE: we do not stack replication requests to targets because this could result in
/// out-of-order delivery. We always buffer until we receive a success response, then send the
/// next payload from the buffer.
pub(crate) struct ReplicationCore<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    /// The ID of the target Raft node which replication events are to be sent to.
    target: C::NodeId,

    /// Identifies which session this replication belongs to.
    session_id: ReplicationSessionId<C::NodeId>,

    /// A channel for sending events to the RaftCore.
    #[allow(clippy::type_complexity)]
    tx_raft_core: mpsc::UnboundedSender<RaftMsg<C, N, S>>,

    /// A channel for receiving events from the RaftCore.
    rx_repl: mpsc::UnboundedReceiver<Replicate<C::NodeId>>,

    /// The `RaftNetwork` interface.
    network: N::Network,

    /// The `RaftLogReader` of a `RaftStorage` interface.
    log_reader: S::LogReader,

    /// The Raft's runtime config.
    config: Arc<Config>,

    /// The target state of this replication stream.
    target_repl_state: TargetReplState,

    /// The log id of the highest log entry which is known to be committed in the cluster.
    committed: Option<LogId<C::NodeId>>,

    /// Replication progress
    progress: ProgressEntry<C::NodeId>,

    /// if or not need to replicate log entries or states, e.g., `commit_index` etc.
    need_to_replicate: bool,
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> ReplicationCore<C, N, S> {
    /// Spawn a new replication task for the target node.
    #[tracing::instrument(level = "trace", skip_all,fields(target=display(target), session_id=display(session_id)))]
    #[allow(clippy::type_complexity)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn(
        target: C::NodeId,
        session_id: ReplicationSessionId<C::NodeId>,
        config: Arc<Config>,
        committed: Option<LogId<C::NodeId>>,
        progress_entry: ProgressEntry<C::NodeId>,
        network: N::Network,
        log_reader: S::LogReader,
        tx_raft_core: mpsc::UnboundedSender<RaftMsg<C, N, S>>,
        span: tracing::Span,
    ) -> ReplicationHandle<C::NodeId> {
        tracing::debug!(
            session_id = display(&session_id),
            target = display(&target),
            committed = display(committed.summary()),
            progress_entry = debug(&progress_entry),
            "spawn replication"
        );
        // other component to ReplicationStream
        let (tx_repl, rx_repl) = mpsc::unbounded_channel();

        let this = Self {
            target,
            session_id,
            network,
            log_reader,
            config,
            target_repl_state: TargetReplState::LineRate,
            committed,
            progress: progress_entry,
            tx_raft_core,
            rx_repl,
            need_to_replicate: true,
        };

        let join_handle = tokio::spawn(this.main().instrument(span));

        ReplicationHandle { join_handle, tx_repl }
    }

    #[tracing::instrument(level="debug", skip(self), fields(session=%self.session_id, target=display(self.target), cluster=%self.config.cluster_name))]
    async fn main(mut self) {
        loop {
            // If it returns Ok(), always go back to LineRate state.
            let res = match &self.target_repl_state {
                TargetReplState::LineRate => self.line_rate_loop().await,
                TargetReplState::Snapshotting => self.replicate_snapshot().await,
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
                    return;
                }
                ReplicationError::HigherVote(h) => {
                    let _ = self.tx_raft_core.send(RaftMsg::HigherVote {
                        target: self.target,
                        higher: h.higher,
                        vote: self.session_id.vote,
                    });
                    return;
                }
                ReplicationError::LackEntry(_lack_ent) => {
                    self.set_target_repl_state(TargetReplState::Snapshotting);
                }
                ReplicationError::CommittedAdvanceTooMany { .. } => {
                    self.set_target_repl_state(TargetReplState::Snapshotting);
                }
                ReplicationError::StorageError(_err) => {
                    // TODO: report this error
                    let _ = self.tx_raft_core.send(RaftMsg::ReplicationFatal);
                    return;
                }
                ReplicationError::RPCError(_e) => {
                    unreachable!("RPCError is dealt with in the inner loop");
                }
            };
        }
    }

    /// Send an AppendEntries RPC to the target.
    ///
    /// This request will timeout if no response is received within the
    /// configured heartbeat interval.
    #[tracing::instrument(level = "debug", skip(self))]
    async fn send_append_entries(&mut self) -> Result<(), ReplicationError<C::NodeId, C::Node>> {
        let (start, _right) = self.progress.sending_start();

        let mut prev_index = if start == 0 { None } else { Some(start - 1) };

        let (prev_log_id, logs, has_more_logs) = loop {
            // TODO(xp): test heartbeat when all logs are removed.

            let log_state = self.log_reader.get_log_state().await?;

            let last_purged = log_state.last_purged_log_id;

            self.check_consecutive(last_purged)?;

            if prev_index < last_purged.index() {
                prev_index = last_purged.index();
            }

            let last_log_index = log_state.last_log_id.next_index();
            let start = prev_index.next_index();
            let end = std::cmp::min(start + self.config.max_payload_entries, last_log_index);

            tracing::debug!(
                progress = display(&self.progress),
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

            break (prev_log_id, logs, end < last_log_index);
        };

        let conflict = prev_log_id;
        let matched = if logs.is_empty() {
            prev_log_id
        } else {
            Some(logs[logs.len() - 1].log_id)
        };

        // Build the heartbeat frame to be sent to the follower.
        let payload = AppendEntriesRequest {
            vote: self.session_id.vote,
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

        let append_res = res.map_err(|_e| {
            let to = Timeout {
                action: RPCTypes::AppendEntries,
                id: self.session_id.vote.node_id,
                target: self.target,
                timeout: the_timeout,
            };
            RPCError::Timeout(to)
        })?;
        let append_resp = append_res?;

        tracing::debug!("append_entries resp: {:?}", append_resp);

        match append_resp {
            AppendEntriesResponse::Success => {
                self.update_matched(matched);

                // Set the need_to_replicate flag if there is more log to send.
                // Otherwise leave it as is.
                self.need_to_replicate = has_more_logs;
                Ok(())
            }
            AppendEntriesResponse::HigherVote(vote) => {
                assert!(
                    vote > self.session_id.vote,
                    "higher vote should be greater than leader's vote"
                );
                tracing::debug!(%vote, "append entries failed. converting to follower");

                Err(ReplicationError::HigherVote(HigherVote {
                    higher: vote,
                    mine: self.session_id.vote,
                }))
            }
            AppendEntriesResponse::Conflict => {
                debug_assert!(conflict.is_some(), "prev_log_id=None never conflict");
                let conflict = conflict.unwrap();
                self.progress.update_conflicting(conflict.index);

                Ok(())
            }
        }
    }

    /// max_possible_matched_index is the least index for `prev_log_id` to form a consecutive log sequence
    #[tracing::instrument(level = "trace", skip_all)]
    fn check_consecutive(&self, last_purged: Option<LogId<C::NodeId>>) -> Result<(), LackEntry<C::NodeId>> {
        tracing::debug!(?last_purged, progress = display(&self.progress), "check_consecutive");

        if last_purged.index() > self.progress.max_possible_matching() {
            return Err(LackEntry {
                index: self.progress.max_possible_matching(),
                last_purged_log_id: last_purged,
            });
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
    fn update_matched(&mut self, new_matched: Option<LogId<C::NodeId>>) {
        tracing::debug!(progress = display(&self.progress), ?new_matched, "update_matched");

        if self.progress.matching < new_matched {
            self.progress.update_matching(new_matched);

            tracing::debug!(target=%self.target, progress=display(&self.progress), "matched updated");

            let _ = self.tx_raft_core.send(RaftMsg::UpdateReplicationProgress {
                session_id: self.session_id,
                target: self.target,
                result: Ok(self.progress),
            });
        }
    }

    /// Perform a check to see if this replication stream is lagging behind far enough that a
    /// snapshot is warranted.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(self) fn needs_snapshot(&self) -> bool {
        let c = self.committed.next_index();
        let m = self.progress.matching.next_index();
        let distance = c.saturating_sub(m);

        #[allow(clippy::infallible_destructuring_match)]
        let snapshot_threshold = match self.config.snapshot_policy {
            SnapshotPolicy::LogsSinceLast(n) => n,
        };

        let lagging_threshold = self.config.replication_lag_threshold;
        let needs_snap = distance >= lagging_threshold && distance > snapshot_threshold;

        tracing::trace!(
            "snapshot needed: {}, distance:{}; lagging_threshold:{}; snapshot_threshold:{}",
            needs_snap,
            distance,
            lagging_threshold,
            snapshot_threshold
        );
        needs_snap
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn try_drain_raft_rx(&mut self) -> Result<(), ReplicationError<C::NodeId, C::Node>> {
        tracing::debug!("try_drain_raft_rx");

        for _i in 0..self.config.max_payload_entries {
            let event_or_nothing = self.rx_repl.recv().now_or_never();
            let ev_opt = match event_or_nothing {
                None => {
                    // no event in self.repl_rx
                    return Ok(());
                }
                Some(x) => x,
            };

            let event = match ev_opt {
                None => {
                    // channel is closed, Leader quited.
                    return Err(ReplicationError::Closed);
                }
                Some(x) => x,
            };

            self.process_raft_event(event)
        }

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip_all)]
    pub fn process_raft_event(&mut self, event: Replicate<C::NodeId>) {
        tracing::debug!(event=%event.summary(), "process_raft_event");

        match event {
            Replicate::Committed(c) => {
                if c > self.committed {
                    self.need_to_replicate = true;
                    self.committed = c;
                }
            }
            Replicate::Entries(last) => {
                if last.index() > self.progress.matching.index() {
                    self.need_to_replicate = true;
                }
            }
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
}

/// An event from the RaftCore leader state to replication stream, to inform what to replicate.
pub(crate) enum Replicate<NID: NodeId> {
    /// Inform replication stream to forward the committed log id to followers/learners.
    Committed(Option<LogId<NID>>),

    /// Inform replication stream to forward the log entries to followers/learners.
    ///
    /// This message contains the last log id on this leader
    Entries(Option<LogId<NID>>),
}

impl<NID: NodeId> MessageSummary<Replicate<NID>> for Replicate<NID> {
    fn summary(&self) -> String {
        match self {
            Replicate::Committed(c) => {
                format!("Replicate::Committed: {:?}", c)
            }
            Replicate::Entries(last) => {
                format!("Replicate::Entries: upto: {:?}", last)
            }
        }
    }
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> ReplicationCore<C, N, S> {
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn line_rate_loop(&mut self) -> Result<(), ReplicationError<C::NodeId, C::Node>> {
        loop {
            loop {
                tracing::debug!("progress: {}", self.progress);

                let res = self.send_append_entries().await;
                tracing::debug!(target = display(self.target), res = debug(&res), "replication res",);

                if let Err(err) = res {
                    tracing::error!(error=%err, "error replication to target={}", self.target);

                    // For transport error, just keep retrying.
                    match err {
                        ReplicationError::RPCError(e) => {
                            tracing::error!(%e, "RPCError");
                            let _ = self.tx_raft_core.send(RaftMsg::UpdateReplicationProgress {
                                target: self.target,
                                result: Err(e.to_string()),
                                session_id: self.session_id,
                            });
                            break;
                        }
                        _ => {
                            return Err(err);
                        }
                    }
                }

                if self.progress.searching.is_none() {
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

            // Check raft channel to ensure we are staying up-to-date
            self.try_drain_raft_rx().await?;
            tracing::debug!(
                target = display(self.target),
                need = display(self.need_to_replicate),
                "need_to_replicate"
            );
            if self.need_to_replicate {
                // if there is more log, continue to send_append_entries
                continue;
            }

            let event_or_none = self.rx_repl.recv().await;
            match event_or_none {
                Some(event) => {
                    self.process_raft_event(event);
                    self.try_drain_raft_rx().await?;
                }
                None => {
                    tracing::debug!("received: RaftEvent::Terminate: closed");
                    return Err(ReplicationError::Closed);
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip(self), fields(state = "snapshotting"))]
    pub async fn replicate_snapshot(&mut self) -> Result<(), ReplicationError<C::NodeId, C::Node>> {
        let snapshot = self.wait_for_snapshot().await?;
        self.stream_snapshot(snapshot).await?;

        Ok(())
    }

    /// Ask RaftCore for a snapshot
    #[tracing::instrument(level = "debug", skip(self))]
    async fn wait_for_snapshot(
        &mut self,
    ) -> Result<Snapshot<C::NodeId, C::Node, S::SnapshotData>, ReplicationError<C::NodeId, C::Node>> {
        // Ask raft core for a snapshot.
        //
        // RaftCore must have a ready snapshot:
        // When `ReplicationCore` asks for a snapshot for replication, `RaftCore`
        // could just gives it the last built one. There is never need to rebuild
        // one. Because a log won't be purged until a snapshot including it is
        // built.

        let (tx, rx) = oneshot::channel();

        let _ = self.tx_raft_core.send(RaftMsg::NeedsSnapshot {
            target: self.target,
            tx,
            session_id: self.session_id,
        });

        let snapshot = rx.await.map_err(|e| {
            tracing::info!("error waiting for snapshot: {}", e);
            ReplicationError::Closed
        })?;

        Ok(snapshot)
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn stream_snapshot(
        &mut self,
        mut snapshot: Snapshot<C::NodeId, C::Node, S::SnapshotData>,
    ) -> Result<(), ReplicationError<C::NodeId, C::Node>> {
        let err_x = || (ErrorSubject::Snapshot(snapshot.meta.signature()), ErrorVerb::Read);

        let mut offset = 0;
        let end = snapshot.snapshot.seek(SeekFrom::End(0)).await.sto_res(err_x)?;
        let mut buf = Vec::with_capacity(self.config.snapshot_max_chunk_size as usize);

        loop {
            // Build the RPC.
            snapshot.snapshot.seek(SeekFrom::Start(offset)).await.sto_res(err_x)?;
            let n_read = snapshot.snapshot.read_buf(&mut buf).await.sto_res(err_x)?;

            let done = (offset + n_read as u64) == end;
            let req = InstallSnapshotRequest {
                vote: self.session_id.vote,
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

            let snap_timeout = if done {
                self.config.install_snapshot_timeout()
            } else {
                self.config.send_snapshot_timeout()
            };

            let res = timeout(snap_timeout, self.network.send_install_snapshot(req)).await;

            let res = match res {
                Ok(outer_res) => match outer_res {
                    Ok(res) => res,
                    Err(err) => {
                        tracing::warn!(error=%err, "error sending InstallSnapshot RPC to target");

                        // Sleep a short time otherwise in test environment it is a dead-loop that never yields.
                        // Because network implementation does not yield.
                        sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                },
                Err(err) => {
                    tracing::warn!(error=%err, "timeout while sending InstallSnapshot RPC to target");

                    // Sleep a short time otherwise in test environment it is a dead-loop that never yields.
                    // Because network implementation does not yield.
                    sleep(Duration::from_millis(10)).await;
                    continue;
                }
            };

            // Handle response conditions.
            if res.vote > self.session_id.vote {
                return Err(ReplicationError::HigherVote(HigherVote {
                    higher: res.vote,
                    mine: self.session_id.vote,
                }));
            }

            // If we just sent the final chunk of the snapshot, then transition to lagging state.
            if done {
                tracing::debug!(
                    "done install snapshot: snapshot last_log_id: {:?}, progress: {}",
                    snapshot.meta.last_log_id,
                    self.progress,
                );

                self.update_matched(snapshot.meta.last_log_id);

                return Ok(());
            }

            // Everything is good, so update offset for sending the next chunk.
            offset += n_read as u64;

            // Check raft channel to ensure we are staying up-to-date, then loop.
            self.try_drain_raft_rx().await?;
        }
    }
}
