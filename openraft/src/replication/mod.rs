//! Replication stream.

pub(crate) mod log_state;
pub(crate) mod replication_context;
mod replication_session_id;
pub(crate) mod replication_state;
pub(crate) mod request;
pub(crate) mod response;
pub(crate) mod snapshot_transmitter;
pub(crate) mod snapshot_transmitter_handle;

use std::sync::Arc;
use std::time::Duration;

use futures::future::FutureExt;
pub(crate) use replication_session_id::ReplicationSessionId;
use replication_state::ReplicationState;
use request::Data;
use request::Replicate;
pub(crate) use response::Progress;
use response::ReplicationResult;
use tracing_futures::Instrument;

use crate::RaftNetworkFactory;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscUnboundedReceiver;
use crate::config::Config;
use crate::core::notification::Notification;
use crate::display_ext::DisplayInstantExt;
use crate::display_ext::DisplayOptionExt;
use crate::entry::RaftEntry;
use crate::entry::raft_entry_ext::RaftEntryExt;
use crate::error::HigherVote;
use crate::error::RPCError;
use crate::error::ReplicationClosed;
use crate::error::ReplicationError;
use crate::error::StorageIOResult;
use crate::error::Timeout;
use crate::log_id::LogIdOptionExt;
use crate::log_id_range::LogIdRange;
use crate::network::Backoff;
use crate::network::RPCOption;
use crate::network::RPCTypes;
use crate::network::v2::RaftNetworkV2;
use crate::progress::inflight_id::InflightId;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::replication::log_state::LogState;
use crate::replication::replication_context::ReplicationContext;
use crate::replication::snapshot_transmitter_handle::SnapshotTransmitterHandle;
use crate::storage::RaftLogReader;
use crate::storage::RaftLogStorage;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::MpscUnboundedReceiverOf;
use crate::type_config::alias::MpscUnboundedSenderOf;
use crate::type_config::async_runtime::mpsc::MpscSender;
use crate::vote::raft_vote::RaftVoteExt;

/// The handle to a spawned replication stream.
pub(crate) struct ReplicationHandle<C>
where C: RaftTypeConfig
{
    pub(crate) session_id: ReplicationSessionId<C>,

    /// The spawn handle of the `ReplicationCore` task.
    pub(crate) join_handle: JoinHandleOf<C, Result<(), ReplicationClosed>>,

    /// The channel used for communicating with the replication task.
    pub(crate) tx_repl: MpscUnboundedSenderOf<C, Replicate<C>>,

    pub(crate) snapshot_transmit_handle: Option<SnapshotTransmitterHandle<C>>,
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
    task_state: ReplicationContext<C>,

    inflight_id: Option<InflightId>,

    /// A channel for receiving events from the RaftCore and snapshot transmitting task.
    rx_event: MpscUnboundedReceiverOf<C, Replicate<C>>,

    /// The `RaftNetwork` interface for replicating logs and heartbeat.
    network: N::Network,

    /// The backoff policy if an [`Unreachable`](`crate::error::Unreachable`) error is returned.
    /// It will be reset to `None` when a successful response is received.
    backoff: Option<Backoff>,

    /// The [`RaftLogStorage::LogReader`] interface.
    log_reader: LS::LogReader,

    /// The log replication state tracking progress and matching logs for the follower.
    replication_sate: ReplicationState<C>,

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
    #[tracing::instrument(level = "trace", skip_all,fields(target=display(&target), session_id=display(&session_id)))]
    #[allow(clippy::type_complexity)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn(
        target: C::NodeId,
        session_id: ReplicationSessionId<C>,
        config: Arc<Config>,
        committed: Option<LogIdOf<C>>,
        matching: Option<LogIdOf<C>>,
        network: N::Network,
        log_reader: LS::LogReader,
        tx_raft_core: MpscSenderOf<C, Notification<C>>,
        span: tracing::Span,
    ) -> ReplicationHandle<C> {
        tracing::debug!(
            session_id = display(&session_id),
            target = display(&target),
            committed = display(committed.display()),
            matching = debug(&matching),
            "spawn replication"
        );

        // Another component to ReplicationStream
        let (tx_event, rx_event) = C::mpsc_unbounded();

        let id = session_id.leader_vote.node_id().clone();

        let this = Self {
            task_state: ReplicationContext {
                id,
                target,
                session_id: session_id.clone(),
                config,
                tx_notify: tx_raft_core,
            },
            inflight_id: None,
            network,
            backoff: None,
            log_reader,
            replication_sate: ReplicationState {
                stream_id: 0,
                purged: None,
                local: LogState { committed, last: None },
                remote: LogState {
                    committed: None,
                    last: matching,
                },
                searching_end: 0,
            },
            rx_event,
            next_action: None,
        };

        let join_handle = C::spawn(this.main().instrument(span));

        ReplicationHandle {
            session_id,
            join_handle,
            tx_repl: tx_event,
            snapshot_transmit_handle: None,
        }
    }

    #[tracing::instrument(level="debug", skip(self), fields(session=%self.task_state.session_id, target=display(&self.task_state.target), cluster=%self.task_state.config.cluster_name))]
    async fn main(mut self) -> Result<(), ReplicationClosed> {
        loop {
            let action = self.next_action.take();

            let Some(d) = action else {
                self.drain_events_with_backoff().await?;
                continue;
            };

            tracing::debug!(replication_data = display(&d), "{} send replication RPC", func_name!());

            // If an RPC response is expected by RaftCore
            let need_notify = d.has_payload();

            let log_id_range = match d {
                Data::Committed => {
                    let m = &self.replication_sate.remote.last;
                    let d = LogIdRange::new(m.clone(), m.clone());
                    self.inflight_id = None;
                    d
                }
                Data::Logs {
                    inflight_id,
                    log_id_range,
                } => {
                    self.inflight_id = Some(inflight_id);
                    log_id_range
                }
            };

            let res = self.send_log_entries(log_id_range).await;

            tracing::debug!(res = debug(&res), "replication action done");

            match res {
                Ok(_) => {
                    // reset backoff at once if replication succeeds
                    self.backoff = None;
                }
                Err(err) => {
                    tracing::warn!(error=%err, "error replication to target={}", self.task_state.target);

                    match err {
                        ReplicationError::Closed(closed) => {
                            return Err(closed);
                        }
                        ReplicationError::HigherVote(h) => {
                            self.task_state
                                .tx_notify
                                .send(Notification::HigherVote {
                                    target: self.task_state.target,
                                    higher: h.higher,
                                    leader_vote: self.task_state.session_id.committed_vote(),
                                })
                                .await
                                .ok();
                            return Ok(());
                        }
                        ReplicationError::StorageError(error) => {
                            tracing::error!(error=%error, "error replication to target={}", self.task_state.target);

                            self.task_state.tx_notify.send(Notification::StorageError { error }).await.ok();
                            return Ok(());
                        }
                        ReplicationError::RPCError(err) => {
                            tracing::error!(err = display(&err), "RPCError");

                            match &err {
                                RPCError::Timeout(_) => {}
                                RPCError::Unreachable(_unreachable) => {
                                    // If there is an [`Unreachable`] error, we will backoff for a
                                    // period of time. Backoff will be reset if there is a
                                    // successful RPC is sent.
                                    if self.backoff.is_none() {
                                        self.backoff = Some(self.network.backoff());
                                    }
                                }
                                RPCError::Network(_) => {}
                                RPCError::RemoteError(_) => {}
                            };

                            // If there is no id, it is a heartbeat and do not need to notify RaftCore
                            if need_notify {
                                self.send_progress_error(err).await;
                            } else {
                                tracing::warn!("heartbeat RPC failed, do not send any response to RaftCore");
                            };
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

    /// Send an AppendEntries RPC to the target.
    ///
    /// This request will time out if no response is received within the
    /// configured heartbeat interval.
    ///
    /// If an RPC is made but not completely finished, it returns the next action expected to do.
    ///
    /// `has_payload` indicates if there are any data(AppendEntries) to send, or it is a heartbeat.
    /// `has_payload` decides if it needs to send back notifications to RaftCore.
    #[tracing::instrument(level = "debug", skip_all)]
    async fn send_log_entries(&mut self, log_ids: LogIdRange<C>) -> Result<(), ReplicationError<C>> {
        tracing::debug!(log_id_range = display(&log_ids), "send_log_entries",);

        // Series of logs to send, and the last log id to send
        let (logs, sending_range) = {
            let rng = &log_ids;

            // The log index start and end to send.
            let (start, end) = {
                let start = rng.prev.next_index();
                let end = rng.last.next_index();

                (start, end)
            };

            if start == end {
                // Heartbeat RPC, no logs to send, last log id is the same as prev_log_id
                let r = LogIdRange::new(rng.prev.clone(), rng.prev.clone());
                (vec![], r)
            } else {
                // limited_get_log_entries will return logs smaller than the range [start, end).
                let logs = self.log_reader.limited_get_log_entries(start, end).await.sto_read_logs()?;

                let first = logs.first().map(|ent| ent.ref_log_id()).unwrap();
                let last = logs.last().map(|ent| ent.log_id()).unwrap();

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
            vote: self.task_state.session_id.vote(),
            prev_log_id: sending_range.prev.clone(),
            leader_commit: self.replication_sate.local.committed.clone(),
            entries: logs,
        };

        // Send the payload.
        tracing::debug!(
            payload = display(&payload),
            now = display(leader_time.display()),
            "start sending append_entries, timeout: {:?}",
            self.task_state.config.heartbeat_interval
        );

        let the_timeout = Duration::from_millis(self.task_state.config.heartbeat_interval);
        let option = RPCOption::new(the_timeout);
        let res = C::timeout(the_timeout, self.network.append_entries(payload, option)).await;

        tracing::debug!("append_entries res: {:?}", res);

        let append_res = res.map_err(|_e| {
            let to = Timeout {
                action: RPCTypes::AppendEntries,
                id: self.task_state.session_id.vote().to_leader_node_id(),
                target: self.task_state.target.clone(),
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
                self.notify_heartbeat_progress(leader_time).await;

                let matching = &sending_range.last;

                // If there is no data sent, no need to respond OK notification.
                if self.inflight_id.is_some() {
                    self.notify_progress(ReplicationResult(Ok(matching.clone()))).await;
                    self.update_next_action_to_send(matching.clone(), log_ids);
                }
                Ok(())
            }
            AppendEntriesResponse::PartialSuccess(matching) => {
                Self::debug_assert_partial_success(&sending_range, &matching);

                self.notify_heartbeat_progress(leader_time).await;

                // If there is no data sent, no need to respond OK notification.
                if self.inflight_id.is_some() {
                    self.notify_progress(ReplicationResult(Ok(matching.clone()))).await;
                    self.update_next_action_to_send(matching.clone(), log_ids);
                }
                Ok(())
            }
            AppendEntriesResponse::HigherVote(vote) => {
                debug_assert!(
                    vote.as_ref_vote() > self.task_state.session_id.vote().as_ref_vote(),
                    "higher vote({}) should be greater than leader's vote({})",
                    vote,
                    self.task_state.session_id.vote(),
                );
                tracing::debug!(%vote, "append entries failed. converting to follower");

                Err(ReplicationError::HigherVote(HigherVote {
                    higher: vote,
                    sender_vote: self.task_state.session_id.vote(),
                }))
            }
            AppendEntriesResponse::Conflict => {
                let conflict = sending_range.prev;
                debug_assert!(conflict.is_some(), "prev_log_id=None never conflict");

                let conflict = conflict.unwrap();

                // Conflict is also a successful replication RPC, because the leadership is acknowledged.
                self.notify_heartbeat_progress(leader_time).await;

                // Conflict should always be sent to RaftCore, ignoring `has_payload`
                // because a heartbeat could also cause a conflict if the follower's state reverts.
                self.notify_progress(ReplicationResult(Err(conflict))).await;

                Ok(())
            }
        }
    }

    /// Send the error result to RaftCore.
    /// RaftCore will then submit another replication command.
    async fn send_progress_error(&mut self, err: RPCError<C>) {
        self.task_state
            .tx_notify
            .send(Notification::ReplicationProgress {
                progress: Progress {
                    target: self.task_state.target.clone(),
                    result: Err(err.to_string()),
                    session_id: self.task_state.session_id.clone(),
                },

                inflight_id: self.inflight_id,
            })
            .await
            .ok();
    }

    /// A successful replication implies a successful heartbeat.
    /// This method notifies [`RaftCore`] with a heartbeat progress.
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    async fn notify_heartbeat_progress(&mut self, sending_time: InstantOf<C>) {
        self.task_state
            .tx_notify
            .send({
                Notification::HeartbeatProgress {
                    session_id: self.task_state.session_id.clone(),
                    target: self.task_state.target.clone(),
                    sending_time,
                }
            })
            .await
            .ok();
    }

    /// Notify RaftCore with the success replication result (log matching or conflict).
    async fn notify_progress(&mut self, replication_result: ReplicationResult<C>) {
        tracing::debug!(
            target = display(self.task_state.target.clone()),
            curr_matching = display(self.replication_sate.remote.last.display()),
            result = display(&replication_result),
            "{}",
            func_name!()
        );

        match &replication_result.0 {
            Ok(matching) => {
                self.replication_sate.remote.last = matching.clone();
            }
            Err(_conflict) => {
                // Conflict is not allowed to be less than the current matching.
            }
        }

        self.task_state
            .tx_notify
            .send({
                Notification::ReplicationProgress {
                    progress: Progress {
                        session_id: self.task_state.session_id.clone(),
                        target: self.task_state.target.clone(),
                        result: Ok(replication_result.clone()),
                    },
                    // If it is None, meaning it is not a response to a request with payload.
                    inflight_id: self.inflight_id,
                }
            })
            .await
            .ok();
    }

    /// Drain all events in the channel in backoff mode, i.e., there was an un-retry-able error and
    /// should not send out anything before the backoff interval expired.
    ///
    /// In the backoff period, we should not send out any RPCs, but we should still receive events
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

            futures::select! {
                _ = sleep.fuse() => {
                    tracing::debug!("backoff timeout");
                    return Ok(());
                }
                recv_res = recv.fuse() => {
                    let event = recv_res.ok_or(ReplicationClosed::new("RaftCore closed replication"))?;
                    self.process_event(event);
                }
            }
        }
    }

    /// Receive and process events from RaftCore until `next_action` is filled.
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
        tracing::debug!(event = display(&event), "process_event");

        match event {
            Replicate::Committed { committed: c } => {
                // RaftCore may send a committed equals to the initial value.
                debug_assert!(
                    c >= self.replication_sate.local.committed,
                    "expect new committed {} > self.committed {}",
                    c.display(),
                    self.replication_sate.local.committed.display()
                );

                self.replication_sate.local.committed = c;

                // If there is no action, fill in an heartbeat action to send committed index.
                if self.next_action.is_none() {
                    self.next_action = Some(Data::new_committed());
                }
            }
            Replicate::Data { data: d } => {
                // TODO: Currently there is at most 1 in flight data. But in future RaftCore may send next data
                //       actions without waiting for the previous to finish.
                debug_assert!(
                    !self.next_action.as_ref().map(|d| d.has_payload()).unwrap_or(false),
                    "there cannot be two actions with payload in flight, curr: {}",
                    self.next_action.as_ref().map(|d| d.to_string()).display()
                );

                self.next_action = Some(d);
            }
        }
    }

    /// If there are more logs to send, it returns a new `Some(Data::Logs)` to send.
    fn update_next_action_to_send(&mut self, matching: Option<LogIdOf<C>>, log_ids: LogIdRange<C>) {
        let next = if matching < log_ids.last {
            Some(Data::new_logs(
                LogIdRange::new(matching, log_ids.last),
                // Safe unwrap: this function is called only when self.inflight_id is Some.
                self.inflight_id.unwrap(),
            ))
        } else {
            None
        };
        self.next_action = next;
    }

    /// Check if partial success result(`matching`) is valid for a given log range to send.
    fn debug_assert_partial_success(to_send: &LogIdRange<C>, matching: &Option<LogIdOf<C>>) {
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
