//! Replication stream.

mod replication_session_id;
mod response;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::io::SeekFrom;
use std::sync::Arc;

use anyerror::AnyError;
use futures::future::FutureExt;
pub(crate) use replication_session_id::ReplicationSessionId;
pub(crate) use response::Response;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;
use tokio::select;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio::time::timeout;
use tokio::time::Duration;
use tokio::time::Instant;
use tracing_futures::Instrument;

use crate::config::Config;
use crate::core::notify::Notify;
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
use crate::ErrorSubject;
use crate::ErrorVerb;
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
    pub(crate) join_handle: JoinHandle<Result<(), ReplicationClosed>>,

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

        let join_handle = tokio::spawn(this.main().instrument(span));

        ReplicationHandle { join_handle, tx_repl }
    }

    #[tracing::instrument(level="debug", skip(self), fields(session=%self.session_id, target=display(self.target), cluster=%self.config.cluster_name))]
    async fn main(mut self) -> Result<(), ReplicationClosed> {
        loop {
            let action = std::mem::replace(&mut self.next_action, None);

            let mut repl_id = 0;

            let res = match action {
                None => Ok(()),
                Some(Data { id, payload: r_action }) => {
                    repl_id = id;
                    match r_action {
                        Payload::Logs(log_id_range) => self.send_log_entries(id, log_id_range).await,
                        Payload::Snapshot(snapshot_rx) => self.stream_snapshot(id, snapshot_rx).await,
                    }
                }
            };

            match res {
                Ok(_) => {
                    // reset backoff
                    self.backoff = None;
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
                            let _ = self.tx_raft_core.send(Notify::Network {
                                response: Response::Progress {
                                    target: self.target,
                                    id: repl_id,
                                    result: Err(err.to_string()),
                                    session_id: self.session_id,
                                },
                            });

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

                self.backoff_drain_events(Instant::now() + duration).await?;
            }

            self.drain_events().await?;
        }
    }

    /// Send an AppendEntries RPC to the target.
    ///
    /// This request will timeout if no response is received within the
    /// configured heartbeat interval.
    #[tracing::instrument(level = "debug", skip_all)]
    async fn send_log_entries(
        &mut self,
        id: u64,
        req: LogIdRange<C::NodeId>,
    ) -> Result<(), ReplicationError<C::NodeId, C::Node>> {
        tracing::debug!(id = display(id), send_req = display(&req), "send_log_entries",);

        let start = req.prev_log_id.next_index();
        let end = req.last_log_id.next_index();

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

        // Build the heartbeat frame to be sent to the follower.
        let payload = AppendEntriesRequest {
            vote: self.session_id.vote,
            prev_log_id: req.prev_log_id,
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
        let option = RPCOption::new(the_timeout);
        let res = timeout(the_timeout, self.network.append_entries(payload, option)).await;

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

        tracing::debug!("append_entries resp: {:?}", append_resp);

        match append_resp {
            AppendEntriesResponse::Success => {
                self.update_matching(id, req.last_log_id);
                Ok(())
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
                let conflict = req.prev_log_id;
                debug_assert!(conflict.is_some(), "prev_log_id=None never conflict");

                let conflict = conflict.unwrap();
                self.update_conflicting(id, conflict);

                Ok(())
            }
        }
    }

    fn update_conflicting(&mut self, id: u64, conflict: LogId<C::NodeId>) {
        tracing::debug!(
            target = display(self.target),
            id = display(id),
            conflict = display(&conflict),
            "update_conflicting"
        );

        let _ = self.tx_raft_core.send({
            Notify::Network {
                response: Response::Progress {
                    session_id: self.session_id,
                    id,
                    target: self.target,
                    result: Ok(ReplicationResult::Conflict(conflict)),
                },
            }
        });
    }

    /// Update the `matched` and `max_possible_matched_index`, which both are for tracking
    /// follower replication(the left and right cursor in a bsearch).
    /// And also report the matched log id to RaftCore to commit an entry etc.
    #[tracing::instrument(level = "trace", skip(self))]
    fn update_matching(&mut self, id: u64, new_matching: Option<LogId<C::NodeId>>) {
        tracing::debug!(
            id = display(id),
            target = display(self.target),
            matching = debug(&new_matching),
            "update_matching"
        );

        debug_assert!(self.matching <= new_matching);

        if self.matching < new_matching {
            self.matching = new_matching;

            let _ = self.tx_raft_core.send({
                Notify::Network {
                    response: Response::Progress {
                        session_id: self.session_id,
                        id,
                        target: self.target,
                        result: Ok(ReplicationResult::Matching(new_matching)),
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
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn backoff_drain_events(&mut self, until: Instant) -> Result<(), ReplicationClosed> {
        let d = until - Instant::now();
        tracing::warn!(
            interval = debug(d),
            "{} backoff mode: drain events without processing them",
            func_name!()
        );

        loop {
            let sleep_duration = until - Instant::now();
            let sleep = sleep(sleep_duration);

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

        let event = self.rx_repl.recv().await.ok_or(ReplicationClosed {})?;
        self.process_event(event);

        self.try_drain_events().await?;

        // No action filled after all events are processed, fill in an action to send committed
        // index.
        if self.next_action.is_none() {
            let m = &self.matching;

            // empty message, just for syncing the committed index
            self.next_action = Some(Data {
                // id==0 will be ignored by RaftCore.
                id: 0,
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
        tracing::debug!("try_drain_raft_rx");

        // Just drain all events in the channel.
        // There should not be more than one `Replicate::Data` event in the channel.
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
                // - if self.next_action is None, it resend an empty AppendEntries request as
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
        id: u64,
        rx: oneshot::Receiver<Option<Snapshot<C>>>,
    ) -> Result<(), ReplicationError<C::NodeId, C::Node>> {
        tracing::info!(id = display(id), "{}", func_name!());

        let snapshot = rx.await.map_err(|e| {
            let io_err = StorageIOError::read_snapshot(None, AnyError::error(e));
            StorageError::IO { source: io_err }
        })?;

        tracing::info!(
            "received snapshot: id={}; meta:{}",
            id,
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

            let res = timeout(snap_timeout, self.network.install_snapshot(req, option)).await;

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
                        sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                },
                Err(err) => {
                    tracing::warn!(error=%err, "timeout while sending InstallSnapshot RPC to target");

                    // If sender is closed, return at once
                    self.try_drain_events().await?;

                    // Sleep a short time otherwise in test environment it is a dead-loop that never
                    // yields. Because network implementation does not yield.
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
                    "done install snapshot: snapshot last_log_id: {:?}, matching: {}",
                    snapshot.meta.last_log_id,
                    self.matching.summary(),
                );

                self.update_matching(id, snapshot.meta.last_log_id);

                return Ok(());
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
    id: u64,
    payload: Payload<C>,
}

impl<C> MessageSummary<Data<C>> for Data<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        match &self.payload {
            Payload::Logs(log_id_range) => {
                format!("Logs{{id={}, {}}}", self.id, log_id_range)
            }
            Payload::Snapshot(_) => {
                format!("Snapshot{{id={}}}", self.id)
            }
        }
    }
}

impl<C> Data<C>
where C: RaftTypeConfig
{
    fn new_logs(id: u64, log_id_range: LogIdRange<C::NodeId>) -> Self {
        Self {
            id,
            payload: Payload::Logs(log_id_range),
        }
    }

    fn new_snapshot(id: u64, snapshot_rx: oneshot::Receiver<Option<Snapshot<C>>>) -> Self {
        Self {
            id,
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

impl<C> Debug for Payload<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
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
    pub(crate) fn logs(id: u64, log_id_range: LogIdRange<C::NodeId>) -> Self {
        Self::Data(Data::new_logs(id, log_id_range))
    }

    pub(crate) fn snapshot(id: u64, snapshot_rx: oneshot::Receiver<Option<Snapshot<C>>>) -> Self {
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
