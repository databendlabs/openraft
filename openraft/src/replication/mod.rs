//! Replication stream.

mod replication_session_id;
mod response;

use std::fmt;
use std::io::SeekFrom;
use std::sync::Arc;
use std::time::Duration;

use anyerror::AnyError;
use futures::future::FutureExt;
pub(crate) use replication_session_id::ReplicationSessionId;
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
use crate::raft::InstallSnapshotRequest;
use crate::storage::RaftLogReader;
use crate::storage::RaftLogStorage;
use crate::storage::Snapshot;
use crate::utime::UTime;
use crate::AsyncRuntime;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::Instant;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StorageIOError;
use crate::ToStorageResult;

/// The handle to a spawned replication stream.
pub(crate) struct ReplicationHandle<C>
where C: RaftTypeConfig
{
    /// The spawn handle the `ReplicationCore` task.
    pub(crate) join_handle: <C::AsyncRuntime as AsyncRuntime>::JoinHandle<Result<(), ReplicationClosed>>,

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
        };

        let join_handle = C::AsyncRuntime::spawn(this.main().instrument(span));

        ReplicationHandle { join_handle, tx_repl }
    }

    #[tracing::instrument(level="debug", skip(self), fields(session=%self.session_id, target=display(self.target), cluster=%self.config.cluster_name))]
    async fn main(mut self) -> Result<(), ReplicationClosed> {
        loop {
            let action = self.next_action.take();

            let mut repl_id = None;

            let res = match action {
                None => Ok(None),
                Some(d) => {
                    tracing::debug!(replication_data = display(&d), "{} send replication RPC", func_name!());

                    repl_id = d.request_id;

                    match d.payload {
                        Payload::Logs(log_id_range) => self.send_log_entries(d.request_id, log_id_range).await,
                        Payload::Snapshot(snapshot_rx) => self.stream_snapshot(d.request_id, snapshot_rx).await,
                    }
                }
            };

            tracing::debug!(res = debug(&res), "replication action done");

            match res {
                Ok(next) => {
                    // reset backoff
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

                            if let Some(request_id) = repl_id {
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

                            // If there is an [`Unreachable`] error, we will backoff for a period of time
                            // Backoff will be reset if there is a successful RPC is sent.
                            if let RPCError::Unreachable(_unreachable) = err {
                                if self.backoff.is_none() {
                                    self.backoff = Some(self.network.backoff());
                                }
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

                self.backoff_drain_events(<C::AsyncRuntime as AsyncRuntime>::Instant::now() + duration).await?;
            }

            self.drain_events().await?;
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
        request_id: Option<u64>,
        log_id_range: LogIdRange<C::NodeId>,
    ) -> Result<Option<Data<C>>, ReplicationError<C::NodeId, C::Node>> {
        tracing::debug!(
            request_id = display(request_id.display()),
            log_id_range = display(&log_id_range),
            "send_log_entries",
        );

        let start = log_id_range.prev_log_id.next_index();
        let end = log_id_range.last_log_id.next_index();

        let logs = if start == end {
            vec![]
        } else {
            let logs = self.log_reader.try_get_log_entries(start..end).await?;
            debug_assert_eq!(
                logs.len(),
                (end - start) as usize,
                "expect logs {}..{} but got only {} entries",
                start,
                end,
                logs.len()
            );
            logs
        };

        let leader_time = <C::AsyncRuntime as AsyncRuntime>::Instant::now();

        // Build the heartbeat frame to be sent to the follower.
        let payload = AppendEntriesRequest {
            vote: self.session_id.vote,
            prev_log_id: log_id_range.prev_log_id,
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
        let res = C::AsyncRuntime::timeout(the_timeout, self.network.append_entries(payload, option)).await;

        tracing::debug!("append_entries res: {:?}", res);

        let append_res = res.map_err(|_e| {
            let to = Timeout {
                action: RPCTypes::AppendEntries,
                id: self.session_id.vote.leader_id().voted_for().unwrap(),
                target: self.target,
                timeout: the_timeout,
            };
            RPCError::Timeout(to)
        })?;
        let append_resp = append_res?;

        tracing::debug!(
            req = display(&log_id_range),
            resp = display(&append_resp),
            "append_entries resp"
        );

        match append_resp {
            AppendEntriesResponse::Success => {
                self.update_matching(request_id, leader_time, log_id_range.last_log_id);
                Ok(None)
            }
            AppendEntriesResponse::PartialSuccess(matching) => {
                debug_assert!(
                    matching <= log_id_range.last_log_id,
                    "matching ({}) should be <= last_log_id ({})",
                    matching.display(),
                    log_id_range.last_log_id.display()
                );
                debug_assert!(
                    matching.index() <= log_id_range.last_log_id.index(),
                    "matching.index ({}) should be <= last_log_id.index ({})",
                    matching.index().display(),
                    log_id_range.last_log_id.index().display()
                );
                debug_assert!(
                    matching >= log_id_range.prev_log_id,
                    "matching ({}) should be >= prev_log_id ({})",
                    matching.display(),
                    log_id_range.prev_log_id.display()
                );
                debug_assert!(
                    matching.index() >= log_id_range.prev_log_id.index(),
                    "matching.index ({}) should be >= prev_log_id.index ({})",
                    matching.index().display(),
                    log_id_range.prev_log_id.index().display()
                );

                self.update_matching(request_id, leader_time, matching);
                if matching < log_id_range.last_log_id {
                    // TODO(9): an RPC has already been made, it should use a newer time
                    Ok(Some(Data::new_logs(
                        request_id,
                        LogIdRange::new(matching, log_id_range.last_log_id),
                    )))
                } else {
                    Ok(None)
                }
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
                let conflict = log_id_range.prev_log_id;
                debug_assert!(conflict.is_some(), "prev_log_id=None never conflict");

                let conflict = conflict.unwrap();
                self.update_conflicting(request_id, leader_time, conflict);

                Ok(None)
            }
        }
    }

    fn update_conflicting(
        &mut self,
        request_id: Option<u64>,
        leader_time: <C::AsyncRuntime as AsyncRuntime>::Instant,
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
    fn update_matching(
        &mut self,
        request_id: Option<u64>,
        leader_time: <C::AsyncRuntime as AsyncRuntime>::Instant,
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

        debug_assert!(self.matching <= new_matching);

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
    pub async fn backoff_drain_events(
        &mut self,
        until: <C::AsyncRuntime as AsyncRuntime>::Instant,
    ) -> Result<(), ReplicationClosed> {
        let d = until - <C::AsyncRuntime as AsyncRuntime>::Instant::now();
        tracing::warn!(
            interval = debug(d),
            "{} backoff mode: drain events without processing them",
            func_name!()
        );

        loop {
            let sleep_duration = until - <C::AsyncRuntime as AsyncRuntime>::Instant::now();
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
            self.next_action = Some(Data {
                // request_id==None will be ignored by RaftCore.
                request_id: None,
                payload: Payload::Logs(LogIdRange {
                    prev_log_id: *m,
                    last_log_id: *m,
                }),
            });
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
        request_id: Option<u64>,
        rx: oneshot::Receiver<Option<Snapshot<C>>>,
    ) -> Result<Option<Data<C>>, ReplicationError<C::NodeId, C::Node>> {
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
        let mut buf = Vec::with_capacity(self.config.snapshot_max_chunk_size as usize);

        loop {
            // Build the RPC.
            snapshot.snapshot.seek(SeekFrom::Start(offset)).await.sto_res(err_x)?;
            let n_read = snapshot.snapshot.read_buf(&mut buf).await.sto_res(err_x)?;

            let leader_time = <C::AsyncRuntime as AsyncRuntime>::Instant::now();

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
                self.update_matching(request_id, leader_time, snapshot.meta.last_log_id);

                return Ok(None);
            }

            // Everything is good, so update offset for sending the next chunk.
            offset += n_read as u64;

            // Check raft channel to ensure we are staying up-to-date, then loop.
            self.try_drain_events().await?;
        }
    }
}

/// Request to replicate a chunk of data, logs or snapshot.
///
/// It defines what data to send to a follower/learner and an id to identify who is sending this
/// data.
#[derive(Debug)]
pub(crate) struct Data<C>
where C: RaftTypeConfig
{
    /// The id of this replication request.
    ///
    /// A replication request without an id does not need to send a reply to the caller.
    request_id: Option<u64>,

    payload: Payload<C>,
}

impl<C: RaftTypeConfig> fmt::Display for Data<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{id: {}, payload: {}}}", self.request_id.display(), self.payload)
    }
}

impl<C> MessageSummary<Data<C>> for Data<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        match &self.payload {
            Payload::Logs(log_id_range) => {
                format!("Logs{{request_id={}, {}}}", self.request_id.display(), log_id_range)
            }
            Payload::Snapshot(_) => {
                format!("Snapshot{{request_id={}}}", self.request_id.display())
            }
        }
    }
}

impl<C> Data<C>
where C: RaftTypeConfig
{
    fn new_logs(request_id: Option<u64>, log_id_range: LogIdRange<C::NodeId>) -> Self {
        Self {
            request_id,
            payload: Payload::Logs(log_id_range),
        }
    }

    fn new_snapshot(request_id: Option<u64>, snapshot_rx: oneshot::Receiver<Option<Snapshot<C>>>) -> Self {
        Self {
            request_id,
            payload: Payload::Snapshot(snapshot_rx),
        }
    }
}

/// The data to replication.
///
/// Either a series of logs or a snapshot.
pub(crate) enum Payload<C>
where C: RaftTypeConfig
{
    Logs(LogIdRange<C::NodeId>),
    Snapshot(oneshot::Receiver<Option<Snapshot<C>>>),
}

impl<C> fmt::Display for Payload<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Logs(log_id_range) => {
                write!(f, "Logs({})", log_id_range)
            }
            Self::Snapshot(_) => {
                write!(f, "Snapshot()")
            }
        }
    }
}

impl<C> fmt::Debug for Payload<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Logs(log_id_range) => {
                write!(f, "Logs({})", log_id_range)
            }
            Self::Snapshot(_) => {
                write!(f, "Snapshot()")
            }
        }
    }
}

/// Result of an replication action.
#[derive(Clone, Debug)]
pub(crate) enum ReplicationResult<NID: NodeId> {
    Matching(Option<LogId<NID>>),
    Conflict(LogId<NID>),
}

/// A replication request sent by RaftCore leader state to replication stream.
pub(crate) enum Replicate<C>
where C: RaftTypeConfig
{
    /// Inform replication stream to forward the committed log id to followers/learners.
    Committed(Option<LogId<C::NodeId>>),

    /// Send an empty AppendEntries RPC as heartbeat.
    Heartbeat,

    /// Send a chunk of data, e.g., logs or snapshot.
    Data(Data<C>),
}

impl<C> Replicate<C>
where C: RaftTypeConfig
{
    pub(crate) fn logs(id: Option<u64>, log_id_range: LogIdRange<C::NodeId>) -> Self {
        Self::Data(Data::new_logs(id, log_id_range))
    }

    pub(crate) fn snapshot(id: Option<u64>, snapshot_rx: oneshot::Receiver<Option<Snapshot<C>>>) -> Self {
        Self::Data(Data::new_snapshot(id, snapshot_rx))
    }
}

impl<C> MessageSummary<Replicate<C>> for Replicate<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        match self {
            Replicate::Committed(c) => {
                format!("Replicate::Committed: {:?}", c)
            }
            Replicate::Heartbeat => "Replicate::Heartbeat".to_string(),
            Replicate::Data(d) => {
                format!("Replicate::Data({})", d.summary())
            }
        }
    }
}
