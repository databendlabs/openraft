//! Replication stream.

pub(crate) mod hint;
mod replication_session_id;
pub(crate) mod request;
pub(crate) mod response;

use std::io::SeekFrom;
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
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;
use tokio::select;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing_futures::Instrument;

use crate::config::Config;
use crate::core::notify::Notify;
use crate::display_ext::DisplayOption;
use crate::display_ext::DisplayOptionExt;
use crate::error::HigherVote;
use crate::error::Infallible;
use crate::error::PayloadTooLarge;
use crate::error::RPCError;
use crate::error::RaftError;
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
use crate::raft::InstallSnapshotRequest;
use crate::replication::hint::ReplicationHint;
use crate::storage::RaftLogReader;
use crate::storage::RaftLogStorage;
use crate::storage::Snapshot;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::JoinHandleOf;
use crate::utime::UTime;
use crate::AsyncRuntime;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::Instant;
use crate::LogId;
use crate::MessageSummary;
use crate::RaftLogId;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StorageIOError;
use crate::ToStorageResult;

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

    /// A channel for receiving events from the RaftCore.
    rx_repl: mpsc::UnboundedReceiver<Replicate<C>>,

    /// The `RaftNetwork` interface.
    network: N::Network,

    /// The backoff policy if an [`Unreachable`](`crate::error::Unreachable`) error is returned.
    /// It will be reset to `None` when an successful response is received.
    backoff: Option<Backoff>,

    /// The `RaftLogReader` of a `RaftStorage` interface.
    log_reader: LS::LogReader,

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
    #[tracing::instrument(level = "trace", skip_all,fields(target=display(target), session_id=display(session_id)))]
    #[allow(clippy::type_complexity)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn(
        target: C::NodeId,
        session_id: ReplicationSessionId<C::NodeId>,
        config: Arc<Config>,
        committed: Option<LogId<C::NodeId>>,
        matching: Option<LogId<C::NodeId>>,
        network: N::Network,
        log_reader: LS::LogReader,
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
        let (tx_repl, rx_repl) = mpsc::unbounded_channel();

        let this = Self {
            target,
            session_id,
            network,
            backoff: None,
            log_reader,
            config,
            committed,
            matching,
            tx_raft_core,
            rx_repl,
            next_action: None,
            entries_hint: Default::default(),
        };

        let join_handle = C::AsyncRuntime::spawn(this.main().instrument(span));

        ReplicationHandle { join_handle, tx_repl }
    }

    #[tracing::instrument(level="debug", skip(self), fields(session=%self.session_id, target=display(self.target), cluster=%self.config.cluster_name))]
    async fn main(mut self) -> Result<(), ReplicationClosed> {
        loop {
            let action = self.next_action.take();

            let mut repl_id = None;
            // Backup the log data for retrying.
            let mut log_data = None;

            let res = match action {
                None => Ok(None),
                Some(d) => {
                    tracing::debug!(replication_data = display(&d), "{} send replication RPC", func_name!());

                    repl_id = d.request_id();

                    match d {
                        Data::Logs(log) => {
                            log_data = Some(log.clone());
                            self.send_log_entries(log).await
                        }
                        Data::Snapshot(snap) => self.stream_snapshot(snap).await,
                    }
                }
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
                                    vote: self.session_id.vote,
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
                                self.send_progress_error(repl_id, err);
                            }
                        }
                    };
                }
            };

            if let Some(b) = &mut self.backoff {
                let duration = b.next().unwrap_or_else(|| {
                    tracing::warn!("backoff exhausted, using default");
                    Duration::from_millis(500)
                });

                self.backoff_drain_events(InstantOf::<C>::now() + duration).await?;
            }

            self.drain_events().await?;
        }
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
            request_id = display(request_id.display()),
            log_id_range = display(log_ids.data()),
            "send_log_entries",
        );

        // Series of logs to send, and the last log id to send
        let (logs, sending_range) = {
            let rng = log_ids.data();

            // The log index start and end to send.
            let (start, end) = {
                let start = rng.prev_log_id.next_index();
                let end = rng.last_log_id.next_index();

                if let Some(hint) = self.entries_hint.get() {
                    let hint_end = start + hint;
                    (start, std::cmp::min(end, hint_end))
                } else {
                    (start, end)
                }
            };

            if start == end {
                // Heartbeat RPC, no logs to send, last log id is the same as prev_log_id
                let r = LogIdRange::new(rng.prev_log_id, rng.prev_log_id);
                (vec![], r)
            } else {
                let logs = self.log_reader.try_get_log_entries(start..end).await?;
                debug_assert_eq!(
                    logs.len(),
                    (end - start) as usize,
                    "expect logs {}..{} but got only {} entries, first: {}, last: {}",
                    start,
                    end,
                    logs.len(),
                    logs.first().map(|ent| ent.get_log_id()).display(),
                    logs.last().map(|ent| ent.get_log_id()).display()
                );

                let last_log_id = logs.last().map(|ent| *ent.get_log_id());

                let r = LogIdRange::new(rng.prev_log_id, last_log_id);
                (logs, r)
            }
        };

        let leader_time = InstantOf::<C>::now();

        // Build the heartbeat frame to be sent to the follower.
        let payload = AppendEntriesRequest {
            vote: self.session_id.vote,
            prev_log_id: sending_range.prev_log_id,
            leader_commit: self.committed,
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
        let res = AsyncRuntimeOf::<C>::timeout(the_timeout, self.network.append_entries(payload, option)).await;

        tracing::debug!("append_entries res: {:?}", res);

        let append_res = res.map_err(|_e| {
            let to = Timeout {
                action: RPCTypes::AppendEntries,
                id: self.session_id.vote.leader_id().voted_for().unwrap(),
                target: self.target,
                timeout: the_timeout,
            };
            RPCError::Timeout(to)
        })?; // return Timeout error

        let append_resp = append_res?;

        tracing::debug!(
            req = display(&sending_range),
            resp = display(&append_resp),
            "append_entries resp"
        );

        match append_resp {
            AppendEntriesResponse::Success => {
                let matching = sending_range.last_log_id;
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
                    vote > self.session_id.vote,
                    "higher vote({}) should be greater than leader's vote({})",
                    vote,
                    self.session_id.vote,
                );
                tracing::debug!(%vote, "append entries failed. converting to follower");

                Err(ReplicationError::HigherVote(HigherVote {
                    higher: vote,
                    mine: self.session_id.vote,
                }))
            }
            AppendEntriesResponse::Conflict => {
                let conflict = sending_range.prev_log_id;
                debug_assert!(conflict.is_some(), "prev_log_id=None never conflict");

                let conflict = conflict.unwrap();
                self.send_progress_conflicting(request_id, leader_time, conflict);

                Ok(None)
            }
        }
    }

    /// Send the error result to RaftCore.
    /// RaftCore will then submit another replication command.
    fn send_progress_error(
        &mut self,
        request_id: Option<u64>,
        err: RPCError<C::NodeId, C::Node, RaftError<C::NodeId, Infallible>>,
    ) {
        if let Some(request_id) = request_id {
            let _ = self.tx_raft_core.send(Notify::Network {
                response: Response::Progress {
                    target: self.target,
                    request_id,
                    result: Err(err.to_string()),
                    session_id: self.session_id,
                },
            });
        } else {
            tracing::warn!(
                err = display(&err),
                "encountered RPCError but request_id is None, no response is sent"
            );
        }
    }

    /// Send a `conflict` message to RaftCore.
    /// RaftCore will then submit another replication command.
    fn send_progress_conflicting(
        &mut self,
        request_id: Option<u64>,
        leader_time: InstantOf<C>,
        conflict: LogId<C::NodeId>,
    ) {
        tracing::debug!(
            target = display(self.target),
            request_id = display(request_id.display()),
            conflict = display(&conflict),
            "update_conflicting"
        );

        if let Some(request_id) = request_id {
            let _ = self.tx_raft_core.send({
                Notify::Network {
                    response: Response::Progress {
                        session_id: self.session_id,
                        request_id,
                        target: self.target,
                        result: Ok(UTime::new(leader_time, ReplicationResult::Conflict(conflict))),
                    },
                }
            });
        } else {
            tracing::info!(
                target = display(self.target),
                request_id = display(request_id.display()),
                conflict = display(&conflict),
                "replication conflict, but request_id is None, no response is sent to RaftCore"
            )
        }
    }

    /// Update the `matching` log id, which is for tracking follower replication, and report the
    /// matched log id to `RaftCore` to commit an entry.
    #[tracing::instrument(level = "trace", skip(self))]
    fn send_progress_matching(
        &mut self,
        request_id: Option<u64>,
        leader_time: InstantOf<C>,
        new_matching: Option<LogId<C::NodeId>>,
    ) {
        tracing::debug!(
            request_id = display(request_id.display()),
            target = display(self.target),
            curr_matching = display(DisplayOption(&self.matching)),
            new_matching = display(DisplayOption(&new_matching)),
            "{}",
            func_name!()
        );

        if cfg!(feature = "loosen-follower-log-revert") {
            if self.matching > new_matching {
                tracing::warn!(
                "follower log is reverted from {} to {}; with 'loosen-follower-log-revert' enabled, this is allowed",
                self.matching.display(),
                new_matching.display(),
            );
            }
        } else {
            debug_assert!(
                self.matching <= new_matching,
                "follower log is reverted from {} to {}",
                self.matching.display(),
                new_matching.display(),
            );
        }

        self.matching = new_matching;

        if let Some(request_id) = request_id {
            let _ = self.tx_raft_core.send({
                Notify::Network {
                    response: Response::Progress {
                        session_id: self.session_id,
                        request_id,
                        target: self.target,
                        result: Ok(UTime::new(leader_time, ReplicationResult::Matching(new_matching))),
                    },
                }
            });
        }
    }

    /// Drain all events in the channel in backoff mode, i.e., there was an un-retry-able error and
    /// should not send out anything before backoff interval expired.
    ///
    /// In the backoff period, we should not send out any RPCs, but we should still receive events,
    /// in case the channel is closed, it should quit at once.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn backoff_drain_events(&mut self, until: InstantOf<C>) -> Result<(), ReplicationClosed> {
        let d = until - InstantOf::<C>::now();
        tracing::warn!(
            interval = debug(d),
            "{} backoff mode: drain events without processing them",
            func_name!()
        );

        loop {
            let sleep_duration = until - InstantOf::<C>::now();
            let sleep = C::AsyncRuntime::sleep(sleep_duration);

            let recv = self.rx_repl.recv();

            tracing::debug!("backoff timeout: {:?}", sleep_duration);

            select! {
                _ = sleep => {
                    tracing::debug!("backoff timeout");
                    return Ok(());
                }
                recv_res = recv => {
                    let event = recv_res.ok_or(ReplicationClosed{})?;
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
            let event = self.rx_repl.recv().await.ok_or(ReplicationClosed {})?;
            self.process_event(event);
        }

        self.try_drain_events().await?;

        // No action filled after all events are processed, fill in an action to send committed
        // index.
        if self.next_action.is_none() {
            let m = &self.matching;

            // empty message, just for syncing the committed index
            // request_id==None will be ignored by RaftCore.
            self.next_action = Some(Data::new_logs(None, LogIdRange::new(*m, *m)));
        }

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn try_drain_events(&mut self) -> Result<(), ReplicationClosed> {
        tracing::debug!("{}", func_name!());

        // Just drain all events in the channel.
        // There should NOT be more than one `Replicate::Data` event in the channel.
        // Looping it just collect all commit events and heartbeat events.
        loop {
            let maybe_res = self.rx_repl.recv().now_or_never();

            let recv_res = match maybe_res {
                None => {
                    // No more events in self.repl_rx
                    return Ok(());
                }
                Some(x) => x,
            };

            let event = recv_res.ok_or(ReplicationClosed {})?;

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
            }
            Replicate::Heartbeat => {
                // Nothing to do. Heartbeat message is just for waking up replication to send
                // something: When all messages are drained,
                // - if self.next_action is None, it sends an empty AppendEntries request as
                //   heartbeat.
                //-  If self.next_action is not None, next_action will serve as a heartbeat.
            }
            Replicate::Data(d) => {
                // TODO: Currently there is at most 1 in flight data. But in future RaftCore may send next data
                //       actions without waiting for the previous to finish.
                debug_assert!(self.next_action.is_none(), "there can not be two data action in flight");
                self.next_action = Some(d);
            }
        }
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn stream_snapshot(
        &mut self,
        snapshot_rx: DataWithId<oneshot::Receiver<Option<Snapshot<C>>>>,
    ) -> Result<Option<Data<C>>, ReplicationError<C::NodeId, C::Node>> {
        let request_id = snapshot_rx.request_id();
        let rx = snapshot_rx.into_data();

        tracing::info!(request_id = display(request_id.display()), "{}", func_name!());

        let snapshot = rx.await.map_err(|e| {
            let io_err = StorageIOError::read_snapshot(None, AnyError::error(e));
            StorageError::IO { source: io_err }
        })?;

        tracing::info!(
            "received snapshot: request_id={}; meta:{}",
            request_id.display(),
            snapshot.as_ref().map(|x| &x.meta).summary()
        );

        let mut snapshot = match snapshot {
            None => {
                let io_err = StorageIOError::read_snapshot(None, AnyError::error("snapshot not found"));
                let sto_err = StorageError::IO { source: io_err };
                return Err(ReplicationError::StorageError(sto_err));
            }
            Some(x) => x,
        };

        let err_x = || (ErrorSubject::Snapshot(Some(snapshot.meta.signature())), ErrorVerb::Read);

        let mut offset = 0;
        let end = snapshot.snapshot.seek(SeekFrom::End(0)).await.sto_res(err_x)?;

        loop {
            // Build the RPC.
            snapshot.snapshot.seek(SeekFrom::Start(offset)).await.sto_res(err_x)?;

            let mut buf = Vec::with_capacity(self.config.snapshot_max_chunk_size as usize);
            while buf.capacity() > buf.len() {
                let n = snapshot.snapshot.read_buf(&mut buf).await.sto_res(err_x)?;
                if n == 0 {
                    break;
                }
            }

            let n_read = buf.len();

            let leader_time = InstantOf::<C>::now();

            let done = (offset + n_read as u64) == end;
            let req = InstallSnapshotRequest {
                vote: self.session_id.vote,
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

            let snap_timeout = if done {
                self.config.install_snapshot_timeout()
            } else {
                self.config.send_snapshot_timeout()
            };

            let option = RPCOption::new(snap_timeout);

            let res = C::AsyncRuntime::timeout(snap_timeout, self.network.install_snapshot(req, option)).await;

            let res = match res {
                Ok(outer_res) => match outer_res {
                    Ok(res) => res,
                    Err(err) => {
                        tracing::warn!(error=%err, "error sending InstallSnapshot RPC to target");

                        // If sender is closed, return at once
                        self.try_drain_events().await?;

                        // Sleep a short time otherwise in test environment it is a dead-loop that
                        // never yields. Because network implementation does
                        // not yield.
                        C::AsyncRuntime::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                },
                Err(err) => {
                    // TODO(2): add backoff when Unreachable is returned
                    tracing::warn!(error=%err, "timeout while sending InstallSnapshot RPC to target");

                    // If sender is closed, return at once
                    self.try_drain_events().await?;

                    // Sleep a short time otherwise in test environment it is a dead-loop that never
                    // yields. Because network implementation does not yield.
                    C::AsyncRuntime::sleep(Duration::from_millis(10)).await;
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
                    "done install snapshot: snapshot last_log_id: {:?}, matching: {}",
                    snapshot.meta.last_log_id,
                    self.matching.summary(),
                );

                // TODO: update leader lease for every successfully sent chunk.
                self.send_progress_matching(request_id, leader_time, snapshot.meta.last_log_id);

                return Ok(None);
            }

            // Everything is good, so update offset for sending the next chunk.
            offset += n_read as u64;

            // Check raft channel to ensure we are staying up-to-date, then loop.
            self.try_drain_events().await?;
        }
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
        self.send_progress_matching(log_ids.request_id(), leader_time, matching);

        if matching < log_ids.data().last_log_id {
            Some(Data::new_logs(
                log_ids.request_id(),
                LogIdRange::new(matching, log_ids.data().last_log_id),
            ))
        } else {
            None
        }
    }

    /// Check if partial success result(`matching`) is valid for a given log range to send.
    fn debug_assert_partial_success(to_send: &LogIdRange<C::NodeId>, matching: &Option<LogId<C::NodeId>>) {
        debug_assert!(
            matching <= &to_send.last_log_id,
            "matching ({}) should be <= last_log_id ({})",
            matching.display(),
            to_send.last_log_id.display()
        );
        debug_assert!(
            matching.index() <= to_send.last_log_id.index(),
            "matching.index ({}) should be <= last_log_id.index ({})",
            matching.index().display(),
            to_send.last_log_id.index().display()
        );
        debug_assert!(
            matching >= &to_send.prev_log_id,
            "matching ({}) should be >= prev_log_id ({})",
            matching.display(),
            to_send.prev_log_id.display()
        );
        debug_assert!(
            matching.index() >= to_send.prev_log_id.index(),
            "matching.index ({}) should be >= prev_log_id.index ({})",
            matching.index().display(),
            to_send.prev_log_id.index().display()
        );
    }
}
