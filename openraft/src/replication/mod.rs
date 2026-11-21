//! Replication stream.

mod backoff_consumer;
pub(crate) mod backoff_state;
pub(crate) mod event_watcher;
pub(crate) mod inflight_append;
pub(crate) mod inflight_append_queue;
pub(crate) mod payload;
pub(crate) mod replicate;
pub(crate) mod replication_context;
pub(crate) mod replication_handle;
pub(crate) mod replication_progress;
mod replication_session_id;
pub(crate) mod response;
pub(crate) mod snapshot_transmitter;
pub(crate) mod snapshot_transmitter_handle;
pub(crate) mod stream_context;
pub(crate) mod stream_state;

#[cfg(test)]
mod clear_plan_test;

use std::fmt;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use display_more::DisplayOptionExt;
use futures_util::FutureExt;
use futures_util::StreamExt;
use payload::Payload;
use replication_progress::ReplicationProgress;
pub(crate) use replication_session_id::ReplicationSessionId;
pub(crate) use response::Progress;

/// Fallback delay used when a [`Backoff`] iterator is exhausted.
pub(crate) const EXHAUSTED_BACKOFF_DELAY: Duration = Duration::from_millis(500);
use response::ReplicationResult;
use stream_state::StreamState;
use tracing::Instrument;

use crate::RaftNetworkFactory;
use crate::RaftTypeConfig;
use crate::async_runtime::Mutex;
use crate::async_runtime::Select2;
use crate::async_runtime::watch::WatchReceiver;
use crate::base::BoxStream;
use crate::core::notification::Notification;
use crate::display_ext::display_instant::DisplayInstantExt;
use crate::errors::RPCError;
use crate::errors::ReplicationClosed;
use crate::log_id_range::LogIdRange;
use crate::network::Backoff;
use crate::network::NetBackoff;
use crate::network::NetStreamAppend;
use crate::network::RPCOption;
use crate::progress::inflight_id::InflightId;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendError;
use crate::raft::AppendResult;
use crate::raft::ConflictHint;
use crate::raft_state::IOId;
use crate::replication::backoff_state::BackoffState;
use crate::replication::event_watcher::EventWatcher;
use crate::replication::inflight_append_queue::InflightAppendQueue;
use crate::replication::replicate::Replicate;
use crate::replication::replication_context::ReplicationContext;
use crate::replication::stream_context::StreamContext;
use crate::storage::RaftLogStorage;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::MutexOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::async_runtime::mpsc::MpscSender;

/// The error rank that a stream session adds when the target accepted none of the entries it sent.
///
/// It exceeds `BACKOFF_RANK_THRESHOLD` (20), as the rank of `Unreachable` does, so the next session
/// waits on the backoff instead of resending the entries at once. A lower rank would never start
/// the backoff, because each `Ok` answer in a session resets the rank to 0.
const NO_PROGRESS_BACKOFF_RANK: u64 = 100;

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
    /// Shared context containing node IDs, session info, and notification channel.
    replication_context: ReplicationContext<C>,

    /// State shared with the request stream generator, protected by a mutex.
    stream_state: Arc<MutexOf<C, StreamState<C, LS>>>,

    /// A channel for receiving events from the RaftCore and snapshot transmitting task.
    event_watcher: EventWatcher<C>,

    /// The next replication payload to send, set when partially completed.
    next_action: Option<Payload<C>>,

    /// Identifies the current in-flight replication batch for progress tracking.
    inflight_id: Option<InflightId>,

    /// The network interface for replicating logs and heartbeat.
    network: Option<N::Network>,

    /// The log replication state tracking progress and matching logs for the follower.
    replication_progress: ReplicationProgress<C>,

    /// Backoff state for rate-limiting retries on persistent errors.
    ///
    /// Encapsulates both the accumulated error rank and the shared backoff iterator
    /// handed to [`StreamState`] via
    /// [`BackoffState::consumer`](crate::replication::backoff_state::BackoffState::consumer).
    /// See [issue #1723](https://github.com/databendlabs/openraft/issues/1723) for the
    /// invariant these two pieces must maintain together.
    backoff_state: BackoffState,
}

impl<C, N, LS> ReplicationCore<C, N, LS>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
{
    /// Spawn a new replication task for the target node.
    #[tracing::instrument(level = "trace", skip_all, fields(context=display(&replication_context)
    ))]
    #[allow(clippy::type_complexity)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn(
        replication_context: ReplicationContext<C>,
        progress: ReplicationProgress<C>,
        network: N::Network,
        log_reader: LS::LogReader,
        event_watcher: EventWatcher<C>,
        backoff_reset_rx: WatchReceiverOf<C, ()>,
        span: tracing::Span,
    ) -> JoinHandleOf<C, Result<(), ReplicationClosed>> {
        tracing::debug!(
            "spawn replication: session_id={}, target={}, committed={}, matching={}",
            replication_context.stream_id,
            replication_context.target,
            progress.local_committed.display(),
            progress.remote_matched.display()
        );

        let backoff_state = BackoffState::new();
        let backoff_consumer = backoff_state.consumer(backoff_reset_rx);

        let this = Self {
            replication_context: replication_context.clone(),
            stream_state: Arc::new(C::mutex(StreamState {
                replication_context,
                event_watcher: event_watcher.clone(),
                log_reader,
                payload: None,
                inflight_id: None,
                backoff_consumer,
                #[cfg(test)]
                after_leader_check: None,
            })),
            inflight_id: None,
            event_watcher,
            network: Some(network),
            replication_progress: progress,
            backoff_state,
            next_action: None,
        };

        C::spawn(this.main().instrument(span))
    }

    /// Creates a stream of AppendEntries requests from the given context.
    fn new_request_stream(stream_context: StreamContext<C, LS>) -> BoxStream<'static, AppendEntriesRequest<C>> {
        let strm = futures_util::stream::unfold(stream_context, Self::next_append_request);
        Box::pin(strm)
    }

    /// Generates the next AppendEntries request and records it in the inflight queue.
    ///
    /// Used as the unfold function for the request stream.
    async fn next_append_request(
        stream_context: StreamContext<C, LS>,
    ) -> Option<(AppendEntriesRequest<C>, StreamContext<C, LS>)> {
        let res = {
            let mut state = stream_context.stream_state.as_ref().lock().await;
            state.next_request().await
        };

        let req = match res {
            Ok(Some(req)) => req,
            Ok(None) => return None,
            Err(err) => {
                let mut fatal_error = stream_context.fatal_error.lock().await;
                *fatal_error = Some(err);
                return None;
            }
        };

        let range = req.log_id_range();
        {
            let mut inflight_queue = stream_context.inflight_append_queue.lock().unwrap();
            inflight_queue.push(range.prev, range.last);
        }

        Some((req, stream_context))
    }

    /// Main replication loop that sends AppendEntries requests and processes responses.
    async fn main(mut self) -> Result<(), ReplicationClosed> {
        // Avoid holding a mut ref to self during streaming.
        let mut network = self.network.take().unwrap();

        // reset the streaming state
        self.next_action = None;
        self.inflight_id = None;

        loop {
            tracing::debug!(
                "ReplicationCore: new stream start, next_action = {:?}",
                self.next_action
            );

            self.ensure_still_leading()?;

            // Kept separate from `on_error` because only this scope holds a reference
            // to `network`, which is needed to construct the `Backoff` iterator.
            // If the network returns None, fall back to the policy configured in `Config::backoff`.
            let config = self.replication_context.config.clone();
            self.reconcile_backoff(|| network.backoff().unwrap_or_else(|| config.build_backoff())).await;

            let payload = self.select_next_payload().await?;

            self.next_action = self.run_stream_session(&mut network, &payload).await?;
        }
    }

    async fn reconcile_backoff(&mut self, backoff_factory: impl FnOnce() -> Backoff) {
        let mut stream_state = self.stream_state.lock().await;

        self.backoff_state.reconcile(|| {
            let backoff = backoff_factory();
            // Discard resets from before this backoff; later resets remain pending.
            stream_state.backoff_consumer.reset_rx.borrow_and_update();
            backoff
        });
    }

    /// Check the two reasons this task must stop before opening another stream: the leader
    /// dropped its handle, or a different leader has taken over locally.
    fn ensure_still_leading(&mut self) -> Result<(), ReplicationClosed> {
        let canceled = self.replication_context.cancel_rx.changed().now_or_never();
        if canceled.map(|x| x.is_err()) == Some(true) {
            tracing::info!("ReplicationCore: canceled, quit");
            return Err(ReplicationClosed::new("canceled"));
        }

        let accepted_io: IOId<C> = self.event_watcher.io_accepted_rx.borrow_watched().clone();
        if self.replication_context.leader_changed(&accepted_io) {
            return Err(ReplicationClosed::new("Leader changed"));
        }

        Ok(())
    }

    /// Decide what the next stream should carry.
    ///
    /// A payload still unfinished is resumed only while its plan is [`RaftCore`]'s current one.
    /// Otherwise adopt the plan published since, as when a reverted log makes a `LogsSince` payload
    /// obsolete: resuming the payload would send entries that may no longer exist. With nothing to
    /// run, block until [`RaftCore`] hands over work.
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    async fn select_next_payload(&mut self) -> Result<Payload<C>, ReplicationClosed> {
        if self.next_action.is_some() {
            // `borrow_and_update()` marks the value as seen; a plain `borrow_watched()`
            // would leave the change notification pending, and `drain_events()` would
            // later observe it and replay the same command, re-running an
            // already-acknowledged payload.
            let current = self.event_watcher.replicate_rx.borrow_and_update().clone();
            if let Some(current) = current
                && self.inflight_id != Some(current.inflight_id)
            {
                tracing::info!(
                    "ReplicationCore replaced current data with inflight id {} with {}",
                    self.inflight_id.display(),
                    current
                );
                self.adopt(current).await;
            }
        }

        while self.next_action.is_none() {
            self.drain_events().await?;
        }

        Ok(self.next_action.take().unwrap())
    }

    /// Run `replicate` from now on, in place of the plan this task ran before.
    ///
    /// A plan without payload clears the earlier plan: [`RaftCore`] waits for this task to confirm
    /// that it no longer reads the log for it, and only then purges the logs that plan could read.
    /// The task adopts a plan only between stream sessions. A session for the earlier plan ends
    /// once its request stream ends, which the stream does at its next request after the plan
    /// changes, and the transport has answered the requests in flight. It is not cut short: a
    /// transport may not survive a request dropped halfway, and a dropped request may take with it
    /// a commit that no later request carries.
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    async fn adopt(&mut self, replicate: Replicate<C>) {
        self.inflight_id = Some(replicate.inflight_id);
        self.next_action = replicate.payload;

        if self.next_action.is_none() {
            self.confirm_clear(replicate.inflight_id).await;
        }
    }

    /// Confirm to [`RaftCore`], answering the clear request `inflight_id`, that this task no longer
    /// reads the log for the plan [`RaftCore`] dropped.
    ///
    /// Only the request stream reads the log, and only while it holds the stream state. Taking the
    /// state waits for a read that the stream has started, which a transport that polls the stream
    /// on a task of its own may still run after the stream session ended. Dropping the payload then
    /// leaves the stream nothing to read.
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    async fn confirm_clear(&mut self, inflight_id: InflightId) {
        {
            let mut stream_state = self.stream_state.lock().await;
            stream_state.payload = None;
        }

        tracing::debug!("ReplicationCore cleared its plan; confirm inflight_id: {}", inflight_id);
        self.replication_context.notify_cleared(inflight_id).await;
    }

    /// Stream `payload` to the target and consume the responses.
    ///
    /// Returns the payload left to send: `None` when `payload` completed, or when the session
    /// was interrupted and RaftCore will reschedule. An `Err` is fatal and ends the task.
    async fn run_stream_session(
        &mut self,
        network: &mut N::Network,
        payload: &Payload<C>,
    ) -> Result<Option<Payload<C>>, ReplicationClosed> {
        {
            let mut stream_state = self.stream_state.lock().await;
            stream_state.payload = Some(payload.clone());
            stream_state.inflight_id = self.inflight_id;
        }

        let inflight_queue = InflightAppendQueue::new();
        let inflight_queue = StdMutex::new(inflight_queue);
        let inflight_queue = Arc::new(inflight_queue);
        let fatal_error = Arc::new(C::mutex(None));

        let stream_context = StreamContext {
            stream_state: self.stream_state.clone(),
            inflight_append_queue: inflight_queue.clone(),
            fatal_error: fatal_error.clone(),
        };

        let req_strm = Self::new_request_stream(stream_context);

        let rpc_timeout = Duration::from_millis(self.replication_context.config.heartbeat_interval);
        let option = RPCOption::new(rpc_timeout);

        let resp_strm_res = network.stream_append(req_strm, option).await;
        // A custom streaming transport may poll the request stream while establishing
        // `stream_append()` and then still return an RPC error. Check the fatal marker here too,
        // because in that case there is no response stream for `handle_response_stream()` to
        // consume.
        if let Some(err) = Self::take_stream_fatal_error(&fatal_error).await {
            return Err(err);
        }

        let resp_strm = match resp_strm_res {
            Ok(resp_strm) => resp_strm,
            Err(rpc_err) => {
                self.backoff_state.on_error(rpc_err.backoff_rank());
                self.send_progress_error(rpc_err, "initiate-stream-replication").await;

                return Ok(None);
            }
        };

        let (acked, continue_res) = self.handle_response_stream(resp_strm, inflight_queue.clone()).await;
        if let Some(err) = Self::take_stream_fatal_error(&fatal_error).await {
            return Err(err);
        }

        // If the stream stops with an un-handleable status, such as conflict,
        // do not continue the payload.
        let remaining = if let Ok(()) = continue_res {
            match acked {
                None => Some(payload.clone()),
                Some(matching) => payload.clone().update_matching(matching),
            }
        } else {
            None
        };

        // The target accepted none of the entries this session sent. Without a backoff, the next
        // session would resend them at once.
        let no_progress = remaining.as_ref() == Some(payload);
        let has_unacked = {
            let inflight_queue = inflight_queue.lock().unwrap();
            !inflight_queue.is_empty()
        };
        if no_progress && has_unacked {
            self.backoff_state.on_error(NO_PROGRESS_BACKOFF_RANK);
        }

        Ok(remaining)
    }

    async fn take_stream_fatal_error(
        fatal_error: &Arc<MutexOf<C, Option<ReplicationClosed>>>,
    ) -> Option<ReplicationClosed> {
        let mut fatal_error = fatal_error.lock().await;
        fatal_error.take()
    }

    /// Consume the response stream, returning its greatest acknowledgement and stop reason.
    async fn handle_response_stream<'s>(
        &mut self,
        resp_strm: BoxStream<'s, Result<AppendResult<C>, RPCError<C>>>,
        inflight_queue: Arc<StdMutex<InflightAppendQueue<C>>>,
    ) -> (Option<Option<LogIdOf<C>>>, Result<(), &'static str>) {
        let mut resp_strm = std::pin::pin!(resp_strm);
        let mut cancel_rx = self.replication_context.cancel_rx.clone();
        let mut acked: Option<Option<LogIdOf<C>>> = None;

        loop {
            let next_resp = resp_strm.next();
            let cancel = cancel_rx.changed();
            let rpc_res = C::cancellable_by(next_resp, cancel).await;
            let Some(rpc_res) = rpc_res else {
                tracing::info!("ReplicationCore: canceled while waiting response");
                return (acked, Err("canceled"));
            };

            let Some(rpc_res) = rpc_res else {
                return (acked, Ok(()));
            };

            tracing::debug!("AppendEntries RPC response: {:?}", rpc_res);

            self.backoff_state.observe(&rpc_res);
            let append_res = match rpc_res {
                Ok(stream_append_res) => stream_append_res,
                Err(rpc_err) => {
                    self.send_progress_error(rpc_err, "stream-replication").await;
                    return (acked, Err("RPCError"));
                }
            };

            match append_res {
                Ok(matching) => {
                    let last_acked_sending_time = {
                        let mut inflight_queue = inflight_queue.lock().unwrap();
                        inflight_queue.drain_acked(&matching)
                    };

                    if let Some(last) = last_acked_sending_time {
                        self.notify_heartbeat_progress(last).await;
                    }

                    acked = acked.max(Some(matching.clone()));

                    let greatest_acked = match &acked {
                        Some(greatest_acked) => greatest_acked.clone(),
                        None => unreachable!("acked was just updated"),
                    };
                    let result = ReplicationResult(Ok(greatest_acked));
                    self.notify_progress(result, None).await;
                }
                Err(append_err) => {
                    match append_err {
                        AppendError::Conflict { log_id, hint } => {
                            let sending_time = {
                                let mut inflight_queue = inflight_queue.lock().unwrap();
                                inflight_queue.drain_conflicted(&log_id)
                            };
                            if let Some(sending_time) = sending_time {
                                self.notify_heartbeat_progress(sending_time).await;
                            }

                            let result = ReplicationResult(Err(log_id));
                            self.notify_progress(result, hint).await;
                        }
                        AppendError::HigherVote(higher) => {
                            self.replication_context
                                .tx_notify
                                .send(Notification::HigherVote {
                                    target: self.replication_context.target.clone(),
                                    higher,
                                    leader_vote: self.replication_context.leader_vote.clone(),
                                })
                                .await
                                .ok();
                        }
                    }

                    return (acked, Err("AppendError"));
                }
            }
        }
    }

    /// Send the error result to RaftCore.
    /// RaftCore will then submit another replication command.
    async fn send_progress_error(&mut self, err: RPCError<C>, when: impl fmt::Display) {
        tracing::warn!(
            "ReplicationCore recv RPCError: {}, when:({}); sending error to RaftCore",
            err,
            when
        );

        // no inflight id means there is no payload is sent, and no one is waiting the response, no need to
        // report.
        if self.inflight_id.is_none() {
            return;
        }
        self.replication_context.notify_progress(Err(err.to_string()), self.inflight_id, None).await;
    }

    /// A successful replication implies a successful heartbeat.
    /// This method notifies [`RaftCore`] with a heartbeat progress.
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    async fn notify_heartbeat_progress(&mut self, sending_time: InstantOf<C>) {
        tracing::debug!("ReplicationCore notify_heartbeat_progress: {}", sending_time.display());
        self.replication_context.notify_heartbeat_progress(sending_time).await;
    }

    /// Notify RaftCore with the success replication result (log matching or conflict).
    ///
    /// `hint` is the conflict hint the target returned with a conflict.
    async fn notify_progress(&mut self, replication_result: ReplicationResult<C>, hint: Option<ConflictHint<C>>) {
        tracing::debug!(
            "{}: target={}, curr_matching={}, result={}, inflight_id={}",
            func_name!(),
            self.replication_context.target,
            self.replication_progress.remote_matched.display(),
            replication_result,
            self.inflight_id.display()
        );

        match &replication_result.0 {
            Ok(matching) => {
                self.replication_progress.remote_matched = matching.clone();

                // No need to notify
                if matching.is_none() && self.inflight_id.is_none() {
                    return;
                }
            }
            Err(_conflict) => {
                // Conflict is not allowed to be less than the current matching.
            }
        }

        // always send Conflict error back, even when the inflight id is None
        // for heartbeat to detect log reversion
        self.replication_context.notify_progress(Ok(replication_result), self.inflight_id, hint).await;
    }

    /// Receive and process events from RaftCore and set `next_action` and `inflight_id`.
    ///
    /// - For log entries: sets `inflight_id = Some(id)` and `next_action = Some(payload)`
    /// - For a clear: sets `inflight_id = Some(id)`, leaves `next_action = None`, and confirms the
    ///   clear to RaftCore, see [`Self::adopt()`]
    /// - For committed update: sets `inflight_id = None` and `next_action =
    ///   Some(empty_range_payload)`
    ///
    /// It blocks until at least one event is received.
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn drain_events(&mut self) -> Result<(), ReplicationClosed> {
        tracing::debug!("drain_events");

        let entries = self.event_watcher.replicate_rx.changed();
        let committed = self.event_watcher.committed_rx.changed();

        let selected = C::select2(entries, committed).await;
        match selected {
            Select2::First(entries_res) => {
                entries_res.map_err(|_e| ReplicationClosed::new("replicate_rx closed"))?;
                // This read delivers the command: `borrow_and_update()` marks it as seen so a
                // value arriving after `changed()` resolved is not delivered a second time.
                let data = self.event_watcher.replicate_rx.borrow_and_update().clone();
                // RaftCore publishes `None` only as the initial value, before any plan.
                if let Some(data) = data {
                    self.adopt(data).await;
                }
            }
            Select2::Second(committed_res) => {
                committed_res.map_err(|_e| ReplicationClosed::new("committed_rx closed"))?;

                // Committed update: create an empty-range payload to sync commit index.
                // This read delivers the event, thus it marks the value as seen.
                let committed = self.event_watcher.committed_rx.borrow_and_update().clone();
                self.replication_progress.local_committed = committed;

                let m = self.replication_progress.remote_matched.clone();
                let log_id_range = LogIdRange::new(m.clone(), m);
                self.inflight_id = None;
                self.next_action = Some(Payload::Probe { log_id_range });
            }
        };

        tracing::debug!(
            "drain_events set: inflight_id={:?}, next_action={:?}",
            self.inflight_id,
            self.next_action
        );

        Ok(())
    }
}
