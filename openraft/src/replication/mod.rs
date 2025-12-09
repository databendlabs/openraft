//! Replication stream.

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

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::StreamExt;
use payload::Payload;
use replication_progress::ReplicationProgress;
pub(crate) use replication_session_id::ReplicationSessionId;
pub(crate) use response::Progress;
use response::ReplicationResult;
use stream_state::StreamState;
use tracing_futures::Instrument;

use crate::RaftNetworkFactory;
use crate::RaftTypeConfig;
use crate::alias::JoinHandleOf;
use crate::async_runtime::Mutex;
use crate::async_runtime::watch::WatchReceiver;
use crate::base::BoxStream;
use crate::config::Config;
use crate::core::SharedRuntimeState;
use crate::core::notification::Notification;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::display_instant::DisplayInstantExt;
use crate::error::RPCError;
use crate::error::ReplicationClosed;
use crate::log_id_range::LogIdRange;
use crate::network::Backoff;
use crate::network::RPCOption;
use crate::network::v2::RaftNetworkV2;
use crate::progress::inflight_id::InflightId;
use crate::raft::AppendEntriesRequest;
use crate::raft::StreamAppendError;
use crate::raft::StreamAppendResult;
use crate::raft_state::IOId;
use crate::replication::event_watcher::EventWatcher;
use crate::replication::inflight_append_queue::InflightAppendQueue;
use crate::replication::replication_context::ReplicationContext;
use crate::replication::stream_context::StreamContext;
use crate::storage::RaftLogStorage;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::MutexOf;
use crate::type_config::async_runtime::mpsc::MpscSender;

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

    /// The `RaftNetwork` interface for replicating logs and heartbeat.
    network: Option<N::Network>,

    /// The log replication state tracking progress and matching logs for the follower.
    replication_progress: ReplicationProgress<C>,

    backoff_rank: u64,

    /// Shared backoff state for rate-limiting retries on persistent errors.
    backoff: Arc<std::sync::Mutex<Option<Backoff>>>,
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
        span: tracing::Span,
        runtime_stats: SharedRuntimeState,
    ) -> JoinHandleOf<C, Result<(), ReplicationClosed>> {
        tracing::debug!(
            "spawn replication: session_id={}, target={}, committed={}, matching={}",
            replication_context.session_id,
            replication_context.target,
            progress.local_committed.display(),
            progress.remote_matched.display()
        );

        let backoff = Arc::new(std::sync::Mutex::new(None));

        let this = Self {
            replication_context: replication_context.clone(),
            stream_state: Arc::new(MutexOf::<C, _>::new(StreamState {
                replication_context,
                event_watcher: event_watcher.clone(),
                log_reader,
                payload: None,
                inflight_id: None,
                leader_committed: None,
                backoff: backoff.clone(),
            })),
            inflight_id: None,
            event_watcher,
            network: Some(network),
            replication_progress: progress,
            backoff_rank: 0,
            backoff: backoff.clone(),
            next_action: None,
        };

        C::spawn(this.main().instrument(span))
    }

    /// Creates a stream of AppendEntries requests from the given context.
    fn new_request_stream(stream_context: StreamContext<C, LS>) -> BoxStream<'static, AppendEntriesRequest<C>> {
        let strm = futures::stream::unfold(stream_context, Self::next_append_request);
        Box::pin(strm)
    }

    /// Generates the next AppendEntries request and records it in the inflight queue.
    ///
    /// Used as the unfold function for the request stream.
    async fn next_append_request(
        stream_context: StreamContext<C, LS>,
    ) -> Option<(AppendEntriesRequest<C>, StreamContext<C, LS>)> {
        let req = {
            let mut state = stream_context.stream_state.as_ref().lock().await;
            state.next_request().await?
        };

        stream_context.inflight_append_queue.push(req.last_log_id());

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

            let canceled = self.replication_context.cancel_rx.changed().now_or_never();
            if canceled.map(|x| x.is_err()) == Some(true) {
                tracing::info!("ReplicationCore: canceled, quit");
                return Err(ReplicationClosed::new("canceled"));
            }

            let accepted_io: IOId<C> = self.event_watcher.io_accepted_rx.borrow_watched().clone();
            let current_leader = accepted_io.leader_id().clone();
            let belonging_leader = self.replication_context.session_id.leader_id().clone();
            if current_leader != belonging_leader {
                tracing::info!(
                    "ReplicationCore: Leader changed from {} to {}, quit replication",
                    belonging_leader,
                    current_leader
                );
                return Err(ReplicationClosed::new("Leader changed"));
            }

            if self.backoff_rank > 20 {
                self.enable_backoff(&mut network);
            } else {
                self.disable_backoff();
            }

            if self.next_action.is_none() {
                self.drain_events().await?;
            } else {
                // If there is new data to send, even when the current data is not yet finished sending
                // discard the current data, start to send the new data.
                // When data is reverted in LogsSince mode, it needs such a mechanism.
                let new_data = self.event_watcher.replicate_rx.borrow_watched().clone();
                if self.inflight_id != Some(new_data.inflight_id) {
                    tracing::info!(
                        "ReplicationCore replaced current data with inflight id {:?} with new {}",
                        self.inflight_id,
                        new_data.inflight_id
                    );
                    self.inflight_id = Some(new_data.inflight_id);
                    self.next_action = Some(new_data.payload);
                }
            }

            let mut payload = self.next_action.take().unwrap();

            {
                let mut stream_state = self.stream_state.lock().await;

                // TODO
                stream_state.payload = Some(payload.clone());
                stream_state.inflight_id = self.inflight_id;
                stream_state.leader_committed = self.event_watcher.committed_rx.borrow_watched().clone()
            }

            let inflight_queue = InflightAppendQueue::new();

            let stream_context = StreamContext {
                stream_state: self.stream_state.clone(),
                inflight_append_queue: inflight_queue.clone(),
            };

            let req_strm = Self::new_request_stream(stream_context);

            let rpc_timeout = Duration::from_millis(self.replication_context.config.heartbeat_interval);
            let option = RPCOption::new(rpc_timeout);

            // TODO: this makes the network poll the io Stream, not good.

            let resp_strm_res = network.stream_append(req_strm, option).await;

            let resp_strm = match resp_strm_res {
                Ok(resp_strm) => resp_strm,
                Err(rpc_err) => {
                    self.backoff_rank += rpc_err.backoff_rank();
                    self.send_progress_error(rpc_err, "initiate-stream-replication").await;

                    continue;
                }
            };

            let res = self.handle_response_stream(resp_strm, inflight_queue).await;

            // Response stream is successfully exhausted.
            if res.is_ok() {
                // if partial success is returned, not all data is exhausted. keep sending
                payload.update_matching(self.replication_progress.remote_matched.clone());
                if payload.len() != Some(0) {
                    self.next_action = Some(payload);
                } else {
                    // Payload is all sent.
                    self.inflight_id = None;
                }
            }
        }
    }

    async fn handle_response_stream<'s>(
        &mut self,
        resp_strm: BoxStream<'s, Result<StreamAppendResult<C>, RPCError<C>>>,
        inflight_queue: InflightAppendQueue<C>,
    ) -> Result<(), &'static str> {
        let mut resp_strm = std::pin::pin!(resp_strm);

        while let Some(rpc_res) = resp_strm.next().await {
            tracing::debug!("AppendEntries RPC response: {:?}", rpc_res);

            let append_res = match rpc_res {
                Ok(stream_append_res) => {
                    self.backoff_rank = 0;
                    stream_append_res
                }
                Err(rpc_err) => {
                    self.backoff_rank += rpc_err.backoff_rank();
                    self.send_progress_error(rpc_err, "stream-replication").await;

                    return Err("RPCError");
                }
            };

            match append_res {
                Ok(matching) => {
                    let last_acked_sending_time = inflight_queue.drain_acked(&matching);

                    if let Some(last) = last_acked_sending_time {
                        self.notify_heartbeat_progress(last).await;
                    }

                    self.replication_progress.remote_matched = matching.clone();

                    self.notify_progress(ReplicationResult(Ok(matching))).await;
                }
                Err(append_err) => {
                    match append_err {
                        StreamAppendError::Conflict(conflict_log_id) => {
                            self.notify_progress(ReplicationResult(Err(conflict_log_id))).await;
                        }
                        StreamAppendError::HigherVote(higher) => {
                            self.replication_context
                                .tx_notify
                                .send(Notification::HigherVote {
                                    target: self.replication_context.target.clone(),
                                    higher,
                                    leader_vote: self.replication_context.session_id.committed_vote(),
                                })
                                .await
                                .ok();
                        }
                    }

                    return Err("AppendError");
                }
            }
        }
        Ok(())
    }

    /// Enables backoff for retries when errors persist.
    fn enable_backoff(&self, network: &mut N::Network) {
        let mut backoff = self.backoff.lock().unwrap();
        if backoff.is_none() {
            *backoff = Some(network.backoff());
        }
    }

    /// Disables backoff after successful communication.
    fn disable_backoff(&self) {
        let mut backoff = self.backoff.lock().unwrap();
        *backoff = None;
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
        self.replication_context
            .tx_notify
            .send(Notification::ReplicationProgress {
                progress: Progress {
                    target: self.replication_context.target.clone(),
                    result: Err(err.to_string()),
                    session_id: self.replication_context.session_id.clone(),
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
        tracing::debug!("ReplicationCore notify_heartbeat_progress: {}", sending_time.display());
        self.replication_context
            .tx_notify
            .send({
                Notification::HeartbeatProgress {
                    session_id: self.replication_context.session_id.clone(),
                    target: self.replication_context.target.clone(),
                    sending_time,
                }
            })
            .await
            .ok();
    }

    /// Notify RaftCore with the success replication result (log matching or conflict).
    async fn notify_progress(&mut self, replication_result: ReplicationResult<C>) {
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
                if matching.is_none() {
                    return;
                }
            }
            Err(_conflict) => {
                // Conflict is not allowed to be less than the current matching.
            }
        }

        // always send Conflict error back, even when the inflight id is None
        // for heartbeat to detect log reversion
        self.replication_context
            .tx_notify
            .send({
                Notification::ReplicationProgress {
                    progress: Progress {
                        session_id: self.replication_context.session_id.clone(),
                        target: self.replication_context.target.clone(),
                        result: Ok(replication_result.clone()),
                    },
                    // If it is None, meaning it is not a response to a request with payload.
                    inflight_id: self.inflight_id,
                }
            })
            .await
            .ok();
    }

    /// Receive and process events from RaftCore and set `next_action` and `inflight_id`.
    ///
    /// - For log entries: sets `inflight_id = Some(id)` and `next_action = Some(payload)`
    /// - For committed update: sets `inflight_id = None` and `next_action =
    ///   Some(empty_range_payload)`
    ///
    /// It blocks until at least one event is received.
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn drain_events(&mut self) -> Result<(), ReplicationClosed> {
        tracing::debug!("drain_events");

        let entries = self.event_watcher.replicate_rx.changed();
        let committed = self.event_watcher.committed_rx.changed();

        futures::select! {
            entries_res = entries.fuse() => {
                entries_res.map_err(|_e| ReplicationClosed::new("replicate_rx closed"))?;
                let data = self.event_watcher.replicate_rx.borrow_watched().clone();
                self.inflight_id = Some(data.inflight_id);
                self.next_action = Some(data.payload);
            }
            committed_res = committed.fuse() => {
                committed_res.map_err(|_e| ReplicationClosed::new("committed_rx closed"))?;

                // Committed update: create an empty-range payload to sync commit index.
                let committed = self.event_watcher.committed_rx.borrow_watched().clone();
                self.replication_progress.local_committed = committed;

                let m = self.replication_progress.remote_matched.clone();
                let log_id_range = LogIdRange::new(m.clone(), m);
                self.inflight_id = None;
                self.next_action = Some(Payload::LogIdRange { log_id_range });
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
