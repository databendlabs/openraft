//! Replication stream.

pub(crate) mod callbacks;
pub(crate) mod hint;
mod replication_session_id;
pub(crate) mod request;
pub(crate) mod request_id;
pub(crate) mod response;

use std::sync::Arc;
use std::time::Duration;

use anyerror::AnyError;
use futures::future::FutureExt;
pub(crate) use replication_session_id::ReplicationSessionId;
use request::Data;
use request::DataWithId;
use request::Replicate;
use response::ReplicationResult;
pub(crate) use response::Response;
use tokio::select;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::Mutex;
use tracing_futures::Instrument;

use crate::config::Config;
use crate::core::notify::Notify;
use crate::core::sm::handle::SnapshotReader;
use crate::display_ext::DisplayOptionExt;
use crate::error::decompose::DecomposeResult;
use crate::error::HigherVote;
use crate::error::PayloadTooLarge;
use crate::error::RPCError;
use crate::error::ReplicationClosed;
use crate::error::ReplicationError;
use crate::error::Timeout;
use crate::log_id::LogIdOptionExt;
use crate::log_id_range::LogIdRange;
use crate::network::Backoff;
use crate::network::RPCOption;
use crate::network::RPCTypes;
use crate::network::RaftNetwork;
use crate::network::RaftNetworkFactory;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::replication::callbacks::SnapshotCallback;
use crate::replication::hint::ReplicationHint;
use crate::replication::request_id::RequestId;
use crate::storage::RaftLogReader;
use crate::storage::RaftLogStorage;
use crate::storage::Snapshot;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::TypeConfigExt;
use crate::AsyncRuntime;
use crate::LogId;
use crate::MessageSummary;
use crate::RaftLogId;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StorageIOError;
use crate::Vote;

/// The handle to a spawned replication stream.
pub(crate) struct ReplicationHandle<C>
where C: RaftTypeConfig
{
    /// The spawn handle the `ReplicationCore` task.
    pub(crate) join_handle: JoinHandleOf<C, Result<(), ReplicationClosed>>,

    /// The channel used for communicating with the replication task.
    pub(crate) tx_repl: mpsc::UnboundedSender<Replicate<C>>,
}

/// A task responsible for sending replication events to a target follower in the Raft cluster.
///
/// NOTE: we do not stack replication requests to targets because this could result in
/// out-of-order delivery. We always buffer until we receive a success response, then send the
/// next payload from the buffer.
pub(crate) struct ReplicationCore<C, N, LS>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
{
    /// The ID of the target Raft node which replication events are to be sent to.
    target: C::NodeId,

    /// Identifies which session this replication belongs to.
    session_id: ReplicationSessionId<C::NodeId>,

    /// A channel for sending events to the RaftCore.
    #[allow(clippy::type_complexity)]
    tx_raft_core: mpsc::UnboundedSender<Notify<C>>,

    /// A channel for receiving events from the RaftCore and snapshot transmitting task.
    rx_event: mpsc::UnboundedReceiver<Replicate<C>>,

    /// A weak reference to the Sender for the separate sending-snapshot task to send callback.
    ///
    /// Because 1) ReplicationCore replies on the `close` event to shutdown.
    /// 2) ReplicationCore holds this tx; It is made a weak so that when
    /// RaftCore drops the only non-weak tx, the Receiver `rx_repl` will be closed.
    weak_tx_event: mpsc::WeakUnboundedSender<Replicate<C>>,

    /// The `RaftNetwork` interface for replicating logs and heartbeat.
    network: N::Network,

    /// Another `RaftNetwork` specific for snapshot replication.
    ///
    /// Snapshot transmitting is a long running task, and is processed in a separate task.
    snapshot_network: Arc<Mutex<N::Network>>,

    /// The current snapshot replication state.
    ///
    /// It includes a cancel signaler and the join handle of the snapshot replication task.
    /// When ReplicationCore is dropped, this Sender is dropped, the snapshot task will be notified
    /// to quit.
    snapshot_state: Option<(oneshot::Sender<()>, JoinHandleOf<C, ()>)>,

    /// The backoff policy if an [`Unreachable`](`crate::error::Unreachable`) error is returned.
    /// It will be reset to `None` when an successful response is received.
    backoff: Option<Backoff>,

    /// The `RaftLogReader` of a `RaftStorage` interface.
    log_reader: LS::LogReader,

    /// The handle to get a snapshot directly from state machine.
    snapshot_reader: SnapshotReader<C>,

    /// The Raft's runtime config.
    config: Arc<Config>,

    /// The log id of the highest log entry which is known to be committed in the cluster.
    committed: Option<LogId<C::NodeId>>,

    /// Last matching log id on a follower/learner
    matching: Option<LogId<C::NodeId>>,

    /// Next replication action to run.
    next_action: Option<Data<C>>,

    /// Appropriate number of entries to send.
    /// This is only used by AppendEntries RPC.
    entries_hint: ReplicationHint,
}

impl<C, N, LS> ReplicationCore<C, N, LS>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
{
    /// Spawn a new replication task for the target node.
    #[tracing::instrument(level = "trace", skip_all,fields(target=display(&target), session_id=display(&session_id)))]
    #[allow(clippy::type_complexity)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn(
        target: C::NodeId,
        session_id: ReplicationSessionId<C::NodeId>,
        config: Arc<Config>,
        committed: Option<LogId<C::NodeId>>,
        matching: Option<LogId<C::NodeId>>,
        network: N::Network,
        snapshot_network: N::Network,
        log_reader: LS::LogReader,
        snapshot_reader: SnapshotReader<C>,
        tx_raft_core: mpsc::UnboundedSender<Notify<C>>,
        span: tracing::Span,
    ) -> ReplicationHandle<C> {
        tracing::debug!(
            session_id = display(&session_id),
            target = display(&target),
            committed = display(committed.summary()),
            matching = debug(&matching),
            "spawn replication"
        );

        // other component to ReplicationStream
        let (tx_event, rx_event) = mpsc::unbounded_channel();

        let this = Self {
            target,
            session_id,
            network,
            snapshot_network: Arc::new(Mutex::new(snapshot_network)),
            snapshot_state: None,
            backoff: None,
            log_reader,
            snapshot_reader,
            config,
            committed,
            matching,
            tx_raft_core,
            rx_event,
            weak_tx_event: tx_event.downgrade(),
            next_action: None,
            entries_hint: Default::default(),
        };

        let join_handle = C::AsyncRuntime::spawn(this.main().instrument(span));

        ReplicationHandle {
            join_handle,
            tx_repl: tx_event,
        }
    }

    #[tracing::instrument(level="debug", skip(self), fields(session=%self.session_id, target=display(&self.target), cluster=%self.config.cluster_name))]
    async fn main(mut self) -> Result<(), ReplicationClosed> {
        loop {
            let action = self.next_action.take();

            let Some(d) = action else {
                self.drain_events_with_backoff().await?;
                continue;
            };

            // Backup the log data for retrying.
            let mut log_data = None;

            tracing::debug!(replication_data = display(&d), "{} send replication RPC", func_name!());

            let request_id = d.request_id();

            let res = match d {
                Data::Heartbeat => {
                    let m = &self.matching;
                    // request_id==None will be ignored by RaftCore.
                    let d = DataWithId::new(RequestId::new_heartbeat(), LogIdRange::new(m.clone(), m.clone()));

                    log_data = Some(d.clone());
                    self.send_log_entries(d).await
                }
                Data::Logs(log) => {
                    log_data = Some(log.clone());
                    self.send_log_entries(log).await
                }
                Data::Snapshot(snap) => self.stream_snapshot(snap).await,
                Data::SnapshotCallback(resp) => self.handle_snapshot_callback(resp),
            };

            tracing::debug!(res = debug(&res), "replication action done");

            match res {
                Ok(next) => {
                    // reset backoff at once if replication succeeds
                    self.backoff = None;

                    // If the RPC was successful but not finished, continue.
                    if let Some(next) = next {
                        self.next_action = Some(next);
                    }
                }
                Err(err) => {
                    tracing::warn!(error=%err, "error replication to target={}", self.target);

                    match err {
                        ReplicationError::Closed(closed) => {
                            return Err(closed);
                        }
                        ReplicationError::HigherVote(h) => {
                            let _ = self.tx_raft_core.send(Notify::Network {
                                response: Response::HigherVote {
                                    target: self.target,
                                    higher: h.higher,
                                    sender_vote: self.session_id.vote_ref().clone(),
                                },
                            });
                            return Ok(());
                        }
                        ReplicationError::StorageError(error) => {
                            tracing::error!(error=%error, "error replication to target={}", self.target);

                            // TODO: report this error
                            let _ = self.tx_raft_core.send(Notify::Network {
                                response: Response::StorageError { error },
                            });
                            return Ok(());
                        }
                        ReplicationError::RPCError(err) => {
                            tracing::error!(err = display(&err), "RPCError");

                            let retry = match &err {
                                RPCError::Timeout(_) => false,
                                RPCError::Unreachable(_unreachable) => {
                                    // If there is an [`Unreachable`] error, we will backoff for a
                                    // period of time. Backoff will be reset if there is a
                                    // successful RPC is sent.
                                    if self.backoff.is_none() {
                                        self.backoff = Some(self.network.backoff());
                                    }
                                    false
                                }
                                RPCError::PayloadTooLarge(too_large) => {
                                    self.update_hint(too_large);

                                    // PayloadTooLarge is a retryable error: retry at once.
                                    self.next_action = Some(Data::Logs(log_data.unwrap()));
                                    true
                                }
                                RPCError::Network(_) => false,
                                RPCError::RemoteError(_) => false,
                            };

                            if retry {
                                debug_assert!(self.next_action.is_some(), "next_action must be Some");
                            } else {
                                self.send_progress_error(request_id, err);
                            }
                        }
                    };
                }
            };

            self.drain_events_with_backoff().await?;
        }
    }

    async fn drain_events_with_backoff(&mut self) -> Result<(), ReplicationClosed> {
        if let Some(b) = &mut self.backoff {
            let duration = b.next().unwrap_or_else(|| {
                tracing::warn!("backoff exhausted, using default");
                Duration::from_millis(500)
            });

            self.backoff_drain_events(C::now() + duration).await?;
        }

        self.drain_events().await?;
        Ok(())
    }

    /// When a [`PayloadTooLarge`] error is received, update the hint for the next several RPC.
    fn update_hint(&mut self, too_large: &PayloadTooLarge) {
        const DEFAULT_ENTRIES_HINT_TTL: u64 = 10;

        match too_large.action() {
            RPCTypes::Vote => {
                unreachable!("Vote RPC should not be too large")
            }
            RPCTypes::AppendEntries => {
                self.entries_hint = ReplicationHint::new(too_large.entries_hint(), DEFAULT_ENTRIES_HINT_TTL);
                tracing::debug!(entries_hint = debug(&self.entries_hint), "updated entries hint");
            }
            RPCTypes::InstallSnapshot => {
                // TODO: handle too large
                tracing::error!("InstallSnapshot RPC is too large, but it is not supported yet");
            }
        }
    }

    /// Send an AppendEntries RPC to the target.
    ///
    /// This request will timeout if no response is received within the
    /// configured heartbeat interval.
    ///
    /// If an RPC is made but not completely finished, it returns the next action expected to do.
    #[tracing::instrument(level = "debug", skip_all)]
    async fn send_log_entries(
        &mut self,
        log_ids: DataWithId<LogIdRange<C::NodeId>>,
    ) -> Result<Option<Data<C>>, ReplicationError<C::NodeId, C::Node>> {
        let request_id = log_ids.request_id();

        tracing::debug!(
            request_id = display(request_id),
            log_id_range = display(log_ids.data()),
            "send_log_entries",
        );

        // Series of logs to send, and the last log id to send
        let (logs, sending_range) = {
            let rng = log_ids.data();

            // The log index start and end to send.
            let (start, end) = {
                let start = rng.prev.next_index();
                let end = rng.last.next_index();

                if let Some(hint) = self.entries_hint.get() {
                    let hint_end = start + hint;
                    (start, std::cmp::min(end, hint_end))
                } else {
                    (start, end)
                }
            };

            if start == end {
                // Heartbeat RPC, no logs to send, last log id is the same as prev_log_id
                let r = LogIdRange::new(rng.prev.clone(), rng.prev.clone());
                (vec![], r)
            } else {
                // limited_get_log_entries will return logs smaller than the range [start, end).
                let logs = self.log_reader.limited_get_log_entries(start, end).await?;

                let first = logs.first().map(|x| x.get_log_id().clone()).unwrap();
                let last = logs.last().map(|x| x.get_log_id().clone()).unwrap();

                debug_assert!(
                    !logs.is_empty() && logs.len() <= (end - start) as usize,
                    "expect logs ⊆ [{}..{}) but got {} entries, first: {}, last: {}",
                    start,
                    end,
                    logs.len(),
                    first,
                    last
                );

                let r = LogIdRange::new(rng.prev.clone(), Some(last));
                (logs, r)
            }
        };

        let leader_time = C::now();

        // Build the heartbeat frame to be sent to the follower.
        let payload = AppendEntriesRequest {
            vote: self.session_id.vote_ref().clone(),
            prev_log_id: sending_range.prev.clone(),
            leader_commit: self.committed.clone(),
            entries: logs,
        };

        // Send the payload.
        tracing::debug!(
            payload=%payload.summary(),
            now = debug(leader_time),
            "start sending append_entries, timeout: {:?}",
            self.config.heartbeat_interval
        );

        let the_timeout = Duration::from_millis(self.config.heartbeat_interval);
        let option = RPCOption::new(the_timeout);
        let res = C::timeout(the_timeout, self.network.append_entries(payload, option)).await;

        tracing::debug!("append_entries res: {:?}", res);

        let append_res = res.map_err(|_e| {
            let to = Timeout {
                action: RPCTypes::AppendEntries,
                id: self.session_id.vote_ref().leader_id().voted_for().unwrap(),
                target: self.target.clone(),
                timeout: the_timeout,
            };
            RPCError::Timeout(to)
        })?; // return Timeout error

        let append_resp = DecomposeResult::<C, _, _>::decompose_infallible(append_res)?;

        tracing::debug!(
            req = display(&sending_range),
            resp = display(&append_resp),
            "append_entries resp"
        );

        match append_resp {
            AppendEntriesResponse::Success => {
                let matching = sending_range.last;
                let next = self.finish_success_append(matching, leader_time, log_ids);
                Ok(next)
            }
            AppendEntriesResponse::PartialSuccess(matching) => {
                Self::debug_assert_partial_success(&sending_range, &matching);
                let next = self.finish_success_append(matching, leader_time, log_ids);
                Ok(next)
            }
            AppendEntriesResponse::HigherVote(vote) => {
                debug_assert!(
                    &vote > self.session_id.vote_ref(),
                    "higher vote({}) should be greater than leader's vote({})",
                    vote,
                    self.session_id.vote_ref(),
                );
                tracing::debug!(%vote, "append entries failed. converting to follower");

                Err(ReplicationError::HigherVote(HigherVote {
                    higher: vote,
                    sender_vote: self.session_id.vote_ref().clone(),
                }))
            }
            AppendEntriesResponse::Conflict => {
                let conflict = sending_range.prev.clone();
                debug_assert!(conflict.is_some(), "prev_log_id=None never conflict");

                let conflict = conflict.unwrap();
                self.send_progress(request_id, ReplicationResult::new(leader_time, Err(conflict)));

                Ok(None)
            }
        }
    }

    /// Send the error result to RaftCore.
    /// RaftCore will then submit another replication command.
    fn send_progress_error(&mut self, request_id: RequestId, err: RPCError<C::NodeId, C::Node>) {
        let _ = self.tx_raft_core.send(Notify::Network {
            response: Response::Progress {
                target: self.target.clone(),
                request_id,
                result: Err(err.to_string()),
                session_id: self.session_id.clone(),
            },
        });
    }

    /// Send the success replication result(log matching or conflict) to RaftCore.
    fn send_progress(&mut self, request_id: RequestId, replication_result: ReplicationResult<C>) {
        tracing::debug!(
            request_id = display(request_id),
            target = display(&self.target),
            curr_matching = display(self.matching.display()),
            result = display(&replication_result),
            "{}",
            func_name!()
        );

        match &replication_result.result {
            Ok(matching) => {
                self.validate_matching(matching.clone());
                self.matching = matching.clone();
            }
            Err(_conflict) => {
                // Conflict is not allowed to be less than the current matching.
            }
        }

        let _ = self.tx_raft_core.send({
            Notify::Network {
                response: Response::Progress {
                    session_id: self.session_id.clone(),
                    request_id,
                    target: self.target.clone(),
                    result: Ok(replication_result),
                },
            }
        });
    }

    /// Validate the value for updating matching log id.
    ///
    /// If the matching log id is reverted to a smaller value:
    /// - log a warning message if [`loosen-follower-log-revert`] feature flag is enabled;
    /// - otherwise panic, consider it as a bug.
    ///
    /// [`loosen-follower-log-revert`]: crate::docs::feature_flags#feature-flag-loosen-follower-log-revert
    fn validate_matching(&self, matching: Option<LogId<C::NodeId>>) {
        if cfg!(feature = "loosen-follower-log-revert") {
            if self.matching > matching {
                tracing::warn!(
                    "follower log is reverted from {} to {}; with 'loosen-follower-log-revert' enabled, this is allowed",
                    self.matching.display(),
                    matching.display(),
                );
            }
        } else {
            debug_assert!(
                self.matching <= matching,
                "follower log is reverted from {} to {}",
                self.matching.display(),
                matching.display(),
            );
        }
    }

    /// Drain all events in the channel in backoff mode, i.e., there was an un-retry-able error and
    /// should not send out anything before backoff interval expired.
    ///
    /// In the backoff period, we should not send out any RPCs, but we should still receive events,
    /// in case the channel is closed, it should quit at once.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn backoff_drain_events(&mut self, until: InstantOf<C>) -> Result<(), ReplicationClosed> {
        let d = until - C::now();
        tracing::warn!(
            interval = debug(d),
            "{} backoff mode: drain events without processing them",
            func_name!()
        );

        loop {
            let sleep_duration = until - C::now();
            let sleep = C::sleep(sleep_duration);

            let recv = self.rx_event.recv();

            tracing::debug!("backoff timeout: {:?}", sleep_duration);

            select! {
                _ = sleep => {
                    tracing::debug!("backoff timeout");
                    return Ok(());
                }
                recv_res = recv => {
                    let event = recv_res.ok_or(ReplicationClosed::new("RaftCore closed replication"))?;
                    self.process_event(event);
                }
            }
        }
    }

    /// Receive and process events from RaftCore, until `next_action` is filled.
    ///
    /// It blocks until at least one event is received.
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn drain_events(&mut self) -> Result<(), ReplicationClosed> {
        tracing::debug!("drain_events");

        // If there is next action to run, do not block waiting for events,
        // instead, just try the best to drain all events.
        if self.next_action.is_none() {
            let event =
                self.rx_event.recv().await.ok_or(ReplicationClosed::new("rx_repl is closed in drain_event()"))?;
            self.process_event(event);
        }

        // Returning from process_event(), next_action is never None.

        self.try_drain_events().await?;

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn try_drain_events(&mut self) -> Result<(), ReplicationClosed> {
        tracing::debug!("{}", func_name!());

        // Just drain all events in the channel.
        // There should NOT be more than one `Replicate::Data` event in the channel.
        // Looping it just collect all commit events and heartbeat events.
        loop {
            let maybe_res = self.rx_event.recv().now_or_never();

            let Some(recv_res) = maybe_res else {
                // No more event found in self.repl_rx
                return Ok(());
            };

            let event = recv_res.ok_or(ReplicationClosed::new("rx_repl is closed in try_drain_event"))?;

            self.process_event(event);
        }
    }

    #[tracing::instrument(level = "trace", skip_all)]
    pub fn process_event(&mut self, event: Replicate<C>) {
        tracing::debug!(event=%event.summary(), "process_event");

        match event {
            Replicate::Committed(c) => {
                // RaftCore may send a committed equals to the initial value.
                debug_assert!(
                    c >= self.committed,
                    "expect new committed {} > self.committed {}",
                    c.summary(),
                    self.committed.summary()
                );

                self.committed = c;

                // If there is no action, fill in an heartbeat action to send committed index.
                if self.next_action.is_none() {
                    self.next_action = Some(Data::new_heartbeat());
                }
            }
            Replicate::Heartbeat => {
                // Never overwrite action with payload.
                if self.next_action.is_none() {
                    self.next_action = Some(Data::new_heartbeat());
                }
            }
            Replicate::Data(d) => {
                // TODO: Currently there is at most 1 in flight data. But in future RaftCore may send next data
                //       actions without waiting for the previous to finish.
                debug_assert!(
                    !self.next_action.as_ref().map(|d| d.has_payload()).unwrap_or(false),
                    "there can not be two actions with payload in flight, curr: {}",
                    self.next_action.as_ref().map(|d| d.to_string()).display()
                );

                if cfg!(debug_assertions) {
                    match &d {
                        Data::SnapshotCallback(_) => {
                            debug_assert!(
                                self.snapshot_state.is_some(),
                                "snapshot state must be Some to receive callback"
                            );
                        }
                        _ => {
                            debug_assert!(
                                self.snapshot_state.is_none(),
                                "can not send other data while sending snapshot"
                            );
                        }
                    }
                }

                self.next_action = Some(d);
            }
        }
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn stream_snapshot(
        &mut self,
        snapshot_req: DataWithId<Option<LogIdOf<C>>>,
    ) -> Result<Option<Data<C>>, ReplicationError<C::NodeId, C::Node>> {
        let request_id = snapshot_req.request_id();

        tracing::info!(request_id = display(request_id), "{}", func_name!());

        let snapshot = self.snapshot_reader.get_snapshot().await.map_err(|reason| {
            tracing::warn!(error = display(&reason), "failed to get snapshot from state machine");
            ReplicationClosed::new(reason)
        })?;

        tracing::info!(
            "received snapshot: request_id={}; meta:{}",
            request_id,
            snapshot.as_ref().map(|x| &x.meta).summary()
        );

        let snapshot = match snapshot {
            None => {
                let io_err = StorageIOError::read_snapshot(None, AnyError::error("snapshot not found"));
                let sto_err = StorageError::IO { source: io_err };
                return Err(ReplicationError::StorageError(sto_err));
            }
            Some(x) => x,
        };

        let mut option = RPCOption::new(self.config.install_snapshot_timeout());
        option.snapshot_chunk_size = Some(self.config.snapshot_max_chunk_size as usize);

        let (tx_cancel, rx_cancel) = oneshot::channel();

        let jh = C::spawn(Self::send_snapshot(
            request_id,
            self.snapshot_network.clone(),
            self.session_id.vote_ref().clone(),
            snapshot,
            option,
            rx_cancel,
            self.weak_tx_event.clone(),
        ));

        // When self.rx_event is dropped:
        // 1) ReplicationCore will return from the main loop;
        // 2) and tx_cancel is dropped;
        // 3) and the snapshot task will be notified.
        self.snapshot_state = Some((tx_cancel, jh));
        Ok(None)
    }

    async fn send_snapshot(
        request_id: RequestId,
        network: Arc<Mutex<N::Network>>,
        vote: Vote<C::NodeId>,
        snapshot: Snapshot<C>,
        option: RPCOption,
        cancel: oneshot::Receiver<()>,
        weak_tx: mpsc::WeakUnboundedSender<Replicate<C>>,
    ) {
        let meta = snapshot.meta.clone();

        let mut net = network.lock().await;

        let start_time = C::now();

        let cancel = async move {
            let _ = cancel.await;
            ReplicationClosed::new("ReplicationCore is dropped")
        };

        let res = net.full_snapshot(vote, snapshot, cancel, option).await;
        if let Err(e) = &res {
            tracing::warn!(error = display(e), "failed to send snapshot");
        }

        let res = res.decompose_infallible();

        if let Some(tx_noty) = weak_tx.upgrade() {
            let data = Data::new_snapshot_callback(request_id, start_time, meta, res);
            let send_res = tx_noty.send(Replicate::new_data(data));
            if send_res.is_err() {
                tracing::warn!("weak_tx failed to send snapshot result to ReplicationCore");
            }
        } else {
            tracing::warn!("weak_tx is dropped, no response is sent to ReplicationCore");
        }
    }

    fn handle_snapshot_callback(
        &mut self,
        callback: DataWithId<SnapshotCallback<C>>,
    ) -> Result<Option<Data<C>>, ReplicationError<C::NodeId, C::Node>> {
        tracing::debug!(
            request_id = debug(callback.request_id()),
            response = display(callback.data()),
            matching = display(self.matching.display()),
            "handle_snapshot_response"
        );

        self.snapshot_state = None;

        let request_id = callback.request_id();
        let SnapshotCallback {
            start_time,
            result,
            snapshot_meta,
        } = callback.into_data();

        let resp = result?;

        // Handle response conditions.
        let sender_vote = self.session_id.vote_ref().clone();
        if resp.vote > sender_vote {
            return Err(ReplicationError::HigherVote(HigherVote {
                higher: resp.vote,
                sender_vote,
            }));
        }

        self.send_progress(
            request_id,
            ReplicationResult::new(start_time, Ok(snapshot_meta.last_log_id)),
        );

        Ok(None)
    }

    /// Update matching and build a return value for a successful append-entries RPC.
    ///
    /// If there are more logs to send, it returns a new `Some(Data::Logs)` to send.
    fn finish_success_append(
        &mut self,
        matching: Option<LogId<C::NodeId>>,
        leader_time: InstantOf<C>,
        log_ids: DataWithId<LogIdRange<C::NodeId>>,
    ) -> Option<Data<C>> {
        self.send_progress(
            log_ids.request_id(),
            ReplicationResult::new(leader_time, Ok(matching.clone())),
        );

        if matching < log_ids.data().last {
            Some(Data::new_logs(
                log_ids.request_id(),
                LogIdRange::new(matching, log_ids.data().last.clone()),
            ))
        } else {
            None
        }
    }

    /// Check if partial success result(`matching`) is valid for a given log range to send.
    fn debug_assert_partial_success(to_send: &LogIdRange<C::NodeId>, matching: &Option<LogId<C::NodeId>>) {
        debug_assert!(
            matching <= &to_send.last,
            "matching ({}) should be <= last_log_id ({})",
            matching.display(),
            to_send.last.display()
        );
        debug_assert!(
            matching.index() <= to_send.last.index(),
            "matching.index ({}) should be <= last_log_id.index ({})",
            matching.index().display(),
            to_send.last.index().display()
        );
        debug_assert!(
            matching >= &to_send.prev,
            "matching ({}) should be >= prev_log_id ({})",
            matching.display(),
            to_send.prev.display()
        );
        debug_assert!(
            matching.index() >= to_send.prev.index(),
            "matching.index ({}) should be >= prev_log_id.index ({})",
            matching.index().display(),
            to_send.prev.index().display()
        );
    }
}
