use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::StreamExt;
use futures_util::TryFutureExt;
use futures_util::stream::FuturesUnordered;
use maplit::btreeset;
use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::ChangeMembers;
use crate::Instant;
use crate::Membership;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::TryRecvError;
use crate::async_runtime::watch::WatchSender;
use crate::base::RaftBatch;
use crate::config::Config;
use crate::config::RuntimeConfig;
use crate::core::ClientResponderQueue;
use crate::core::RuntimeStats;
use crate::core::ServerState;
use crate::core::SharedReplicateBatch;
use crate::core::balancer::Balancer;
use crate::core::core_state::CoreState;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::heartbeat::handle::HeartbeatWorkersHandle;
use crate::core::io_flush_tracking::IoProgressSender;
use crate::core::merged_raft_msg_receiver::BatchRaftMsgReceiver;
use crate::core::notification::Notification;
use crate::core::raft_msg::AppendEntriesTx;
use crate::core::raft_msg::ClientReadTx;
use crate::core::raft_msg::RaftMsg;
use crate::core::raft_msg::ResultSender;
use crate::core::raft_msg::VoteTx;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::core::sm;
use crate::display_ext::DisplayInstantExt;
use crate::display_ext::DisplayOptionExt;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::Engine;
use crate::engine::Respond;
use crate::engine::TargetProgress;
use crate::engine::handler::leader_handler::LeaderHandler;
use crate::engine::leader_log_ids::LeaderLogIds;
use crate::entry::RaftEntry;
use crate::entry::payload::EntryPayload;
use crate::error::AllowNextRevertError;
use crate::error::ClientWriteError;
use crate::error::Fatal;
use crate::error::ForwardToLeader;
use crate::error::Infallible;
use crate::error::InitializeError;
use crate::error::NetworkError;
use crate::error::QuorumNotEnough;
use crate::error::RPCError;
use crate::error::StorageIOResult;
use crate::error::Timeout;
use crate::impls::ProgressResponder;
use crate::log_id::option_raft_log_id_ext::OptionRaftLogIdExt;
use crate::metrics::HeartbeatMetrics;
use crate::metrics::MetricsRecorder;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftMetrics;
use crate::metrics::RaftServerMetrics;
use crate::metrics::ReplicationMetrics;
use crate::metrics::SerdeInstant;
use crate::network::NetStreamAppend;
use crate::network::NetTransferLeader;
use crate::network::NetVote;
use crate::network::RPCOption;
use crate::network::RPCTypes;
use crate::network::RaftNetworkFactory;
use crate::progress::Progress;
use crate::progress::stream_id::StreamId;
use crate::quorum::QuorumSet;
use crate::raft::AppendEntriesRequest;
use crate::raft::ClientWriteResult;
use crate::raft::ReadPolicy;
use crate::raft::StreamAppendError;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::linearizable_read::Linearizer;
use crate::raft::message::TransferLeaderRequest;
use crate::raft::responder::Responder;
use crate::raft::responder::core_responder::CoreResponder;
use crate::raft_state::LogStateReader;
use crate::raft_state::io_state::io_id::IOId;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::replication::ReplicationCore;
use crate::replication::ReplicationSessionId;
use crate::replication::event_watcher::EventWatcher;
use crate::replication::replicate::Replicate;
use crate::replication::replication_context::ReplicationContext;
use crate::replication::replication_handle::ReplicationHandle;
use crate::replication::replication_progress;
use crate::replication::snapshot_transmitter::SnapshotTransmitter;
use crate::runtime::RaftRuntime;
use crate::storage::IOFlushed;
use crate::storage::RaftLogStorage;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::MpscReceiverOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::VoteOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;
use crate::type_config::async_runtime::mpsc::MpscSender;
use crate::vote::RaftLeaderId;
use crate::vote::RaftVote;
use crate::vote::committed::CommittedVote;
use crate::vote::non_committed::UncommittedVote;
use crate::vote::raft_vote::RaftVoteExt;
use crate::vote::vote_status::VoteStatus;

/// The result of applying log entries to state machine.
pub(crate) struct ApplyResult<C: RaftTypeConfig> {
    pub(crate) since: u64,
    pub(crate) end: u64,
    pub(crate) last_applied: LogIdOf<C>,
}

impl<C: RaftTypeConfig> Debug for ApplyResult<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApplyResult")
            .field("since", &self.since)
            .field("end", &self.end)
            .field("last_applied", &self.last_applied)
            .finish()
    }
}

impl<C: RaftTypeConfig> fmt::Display for ApplyResult<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ApplyResult([{}, {}), last_applied={})",
            self.since, self.end, self.last_applied,
        )
    }
}

/// The core type implementing the Raft protocol.
pub struct RaftCore<C, NF, LS>
where
    C: RaftTypeConfig,
    NF: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
{
    /// This node's ID.
    pub(crate) id: C::NodeId,

    /// This node's runtime config.
    pub(crate) config: Arc<Config>,

    pub(crate) runtime_config: Arc<RuntimeConfig>,

    /// Additional state that does not directly affect the consensus.
    pub(crate) core_state: CoreState<C>,

    /// The `RaftNetworkFactory` implementation.
    pub(crate) network_factory: NF,

    /// The [`RaftLogStorage`] implementation.
    pub(crate) log_store: LS,

    /// A controlling handle to the [`RaftStateMachine`] worker.
    ///
    /// [`RaftStateMachine`]: `crate::storage::RaftStateMachine`
    pub(crate) sm_handle: sm::handle::Handle<C>,

    pub(crate) engine: Engine<C>,

    /// Responders to send result back to client when logs are applied.
    pub(crate) client_responders: ClientResponderQueue<CoreResponder<C>>,

    /// A mapping of node IDs the replication state of the target node.
    pub(crate) replications: BTreeMap<C::NodeId, ReplicationHandle<C>>,

    pub(crate) heartbeat_handle: HeartbeatWorkersHandle<C>,

    #[allow(dead_code)]
    pub(crate) tx_api: MpscSenderOf<C, RaftMsg<C>>,
    pub(crate) rx_api: BatchRaftMsgReceiver<C>,

    /// A Sender to send callback by other components to [`RaftCore`], when an action is finished,
    /// such as flushing log to disk, or applying log entries to state machine.
    pub(crate) tx_notification: MpscSenderOf<C, Notification<C>>,

    /// A Receiver to receive callback from other components.
    pub(crate) rx_notification: MpscReceiverOf<C, Notification<C>>,

    /// A Watch channel sender for IO completion notifications from storage callbacks.
    /// This is used by IOFlushed callbacks to report IO completion in a synchronous manner.
    pub(crate) tx_io_completed: WatchSenderOf<C, Result<IOId<C>, StorageError<C>>>,

    /// Broadcasts I/O acceptance before submission to storage.
    ///
    /// Updated when `RaftCore` is about to execute an I/O operation. This allows
    /// observers to prepare for upcoming I/O before it actually happens.
    ///
    /// Note: This is sent when `RaftCore` executes the I/O, not when `Engine` accepts it,
    /// since `Engine` is a pure algorithm implementation without I/O capabilities.
    pub(crate) io_accepted_tx: WatchSenderOf<C, IOId<C>>,

    /// Broadcasts I/O submission progress to replication tasks.
    ///
    /// This enables replication tasks to know which log entries have been submitted
    /// to storage and are safe to read. Updated after each I/O submission completes.
    pub(crate) io_submitted_tx: WatchSenderOf<C, IOId<C>>,

    /// For broadcast committed log id to replication task.
    pub(crate) committed_tx: WatchSenderOf<C, Option<LogIdOf<C>>>,

    pub(crate) tx_metrics: WatchSenderOf<C, RaftMetrics<C>>,
    pub(crate) tx_data_metrics: WatchSenderOf<C, RaftDataMetrics<C>>,
    pub(crate) tx_server_metrics: WatchSenderOf<C, RaftServerMetrics<C>>,
    pub(crate) tx_progress: IoProgressSender<C>,

    /// Runtime statistics for Raft operations.
    ///
    /// Owned directly by RaftCore for lock-free access to most stats.
    /// Only `replicate_batch` is shared with replication tasks via `shared_replicate_batch`.
    pub(crate) runtime_stats: RuntimeStats,

    /// Shared histogram for replication batch sizes.
    ///
    /// This is the only stats field that needs to be shared with replication tasks.
    /// All other stats are updated only by RaftCore.
    pub(crate) shared_replicate_batch: SharedReplicateBatch,

    /// External metrics recorder for exporting metrics to custom backends.
    ///
    /// Defaults to `None`. Applications can install a custom recorder
    /// via [`Raft::set_metrics_recorder`] to collect metrics.
    ///
    /// [`Raft::set_metrics_recorder`]: crate::Raft::set_metrics_recorder
    pub(crate) metrics_recorder: Option<Arc<dyn MetricsRecorder>>,

    pub(crate) span: Span,
}

impl<C, NF, LS> RaftCore<C, NF, LS>
where
    C: RaftTypeConfig,
    NF: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
{
    /// The main loop of the Raft protocol.
    pub(crate) async fn main(mut self, rx_shutdown: OneshotReceiverOf<C, ()>) -> Result<Infallible, Fatal<C>> {
        let span = tracing::span!(parent: &self.span, Level::DEBUG, "main");
        let res = self.do_main(rx_shutdown).instrument(span).await;

        // Flush buffered metrics
        self.flush_metrics();

        // Safe unwrap: res is Result<Infallible, _>
        let err = res.unwrap_err();
        match err {
            Fatal::Stopped => { /* Normal quit */ }
            _ => {
                tracing::error!("RaftCore::main error: {}", err);
            }
        }

        tracing::debug!("update metrics for shutdown");
        {
            let mut curr = self.tx_metrics.borrow_watched().clone();
            curr.state = ServerState::Shutdown;
            curr.running_state = Err(err.clone());

            self.tx_metrics.send(curr).ok();
        }

        tracing::info!("RaftCore shutdown complete");

        Err(err)
    }

    #[tracing::instrument(level = "trace", skip_all, fields(id=display(&self.id), cluster=%self.config.cluster_name
    ))]
    async fn do_main(&mut self, rx_shutdown: OneshotReceiverOf<C, ()>) -> Result<Infallible, Fatal<C>> {
        tracing::debug!("raft node is initializing");

        self.engine.startup();
        // It may not finish running all the commands, if there is a command waiting for a callback.
        self.run_engine_commands().await?;

        // Initialize metrics.
        self.flush_metrics();

        self.runtime_loop(rx_shutdown).await
    }

    /// Handle `is_leader` requests.
    ///
    /// Send heartbeat to all voters. We respond once we have
    /// a quorum of agreement.
    ///
    /// Why:
    /// To ensure linearizability, a read request proposed at time `T1` confirms this node's
    /// leadership to guarantee that all the committed entries proposed before `T1` are present in
    /// this node.
    // TODO: the second condition is such a read request can only read from state machine only when the last log it sees
    //       at `T1` is committed.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    pub(super) async fn handle_ensure_linearizable_read(&mut self, read_policy: ReadPolicy, tx: ClientReadTx<C>) {
        // Setup sentinel values to track when we've received majority confirmation of leadership.

        let resp = {
            let l = self.engine.try_leader_handler();
            let lh = match l {
                Ok(leading_handler) => leading_handler,
                Err(forward) => {
                    tx.send(Err(forward.into())).ok();
                    return;
                }
            };

            let read_log_id = lh.get_read_log_id();

            // TODO: this applied is a little stale when being returned to client.
            //       Fix this when the following heartbeats are replaced with calling RaftNetwork.
            let applied = self.engine.state.io_applied().cloned();

            Linearizer::new(self.id.clone(), read_log_id, applied)
        };

        if read_policy == ReadPolicy::LeaseRead {
            let now = C::now();
            // Check if the lease is expired.
            if let Some(last_quorum_acked_time) = self.last_quorum_acked_time()
                && now < last_quorum_acked_time + self.engine.config.timer_config.leader_lease
            {
                tx.send(Ok(resp)).ok();
                return;
            }
            tracing::debug!("{}: lease expired during lease read", self.id);
            // we may no longer leader so error out early
            let err = ForwardToLeader::empty();
            tx.send(Err(err.into())).ok();
            return;
        }

        let my_id = self.id.clone();
        let my_vote = self.engine.state.vote_ref().clone();
        let ttl = Duration::from_millis(self.config.heartbeat_interval);
        let eff_mem = self.engine.state.membership_state.effective().clone();
        let core_tx = self.tx_notification.clone();

        let mut granted = btreeset! {my_id.clone()};

        // single-node quorum, fast path, return quickly.
        if eff_mem.is_quorum(granted.iter()) {
            tx.send(Ok(resp)).ok();
            return;
        }

        // Spawn parallel requests, all with the standard timeout for heartbeats.
        let mut pending = FuturesUnordered::new();

        let voter_progresses = {
            let l = &self.engine.leader.as_ref().unwrap();
            l.progress.iter().filter(|item| l.progress.is_voter(&item.id) == Some(true))
        };

        for item in voter_progresses {
            let target = item.id.clone();
            let progress = &item.val;

            if target == my_id {
                continue;
            }

            let rpc = AppendEntriesRequest {
                vote: my_vote.clone(),
                prev_log_id: progress.matching().cloned(),
                entries: vec![],
                leader_commit: self.engine.state.committed().cloned(),
            };

            // Safe unwrap(): target is in membership
            let target_node = eff_mem.get_node(&target).unwrap().clone();
            let mut client = self.network_factory.new_client(target.clone(), &target_node).await;

            let option = RPCOption::new(ttl);

            let fu = {
                let my_id = my_id.clone();
                let target = target.clone();

                async move {
                    let input_stream = Box::pin(futures_util::stream::once(async { rpc }));

                    let outer_res = C::timeout(ttl, async {
                        let mut output = client.stream_append(input_stream, option).await?;
                        output.next().await.transpose()
                    })
                    .await;

                    match outer_res {
                        Ok(Ok(Some(stream_result))) => Ok((target, stream_result)),
                        Ok(Ok(None)) => {
                            // Stream returned no response - treat as network error
                            Err((
                                target,
                                RPCError::Network(NetworkError::from_string("stream_append returned no response")),
                            ))
                        }
                        Ok(Err(rpc_err)) => Err((target, rpc_err)),
                        Err(_timeout) => {
                            let timeout_err = Timeout {
                                action: RPCTypes::AppendEntries,
                                id: my_id,
                                target: target.clone(),
                                timeout: ttl,
                            };

                            Err((target, RPCError::Timeout(timeout_err)))
                        }
                    }
                }
            };

            let fu = fu.instrument(tracing::debug_span!("spawn_is_leader", target = target.to_string()));
            let task = C::spawn(fu).map_err(move |err| (target, err));

            pending.push(task);
        }

        let waiting_fu = async move {
            // Handle responses as they return.
            while let Some(res) = pending.next().await {
                let (target, stream_result) = match res {
                    Ok(Ok(res)) => res,
                    Ok(Err((target, err))) => {
                        tracing::error!(
                            "timeout while confirming leadership for read request, target: {}, error: {}",
                            target,
                            err
                        );
                        continue;
                    }
                    Err((target, err)) => {
                        tracing::error!("failed to join task: {}, target: {}", err, target);
                        continue;
                    }
                };

                // If we receive a response with a greater vote, then revert to follower and abort this
                // request.
                if let Err(StreamAppendError::HigherVote(vote)) = stream_result {
                    debug_assert!(
                        vote.as_ref_vote() > my_vote.as_ref_vote(),
                        "committed vote({}) has total order relation with other votes({})",
                        my_vote,
                        vote
                    );

                    let send_res = core_tx
                        .send(Notification::HigherVote {
                            target,
                            higher: vote,
                            leader_vote: my_vote.into_committed(),
                        })
                        .await;

                    if let Err(_e) = send_res {
                        tracing::error!("failed to send HigherVote to RaftCore");
                    }

                    // we are no longer leader so error out early
                    let err = ForwardToLeader::empty();
                    tx.send(Err(err.into())).ok();
                    return;
                }

                // Success or Conflict both confirm leadership (got valid response from follower)
                granted.insert(target);

                if eff_mem.is_quorum(granted.iter()) {
                    tx.send(Ok(resp)).ok();
                    return;
                }
            }

            // If we've hit this location, then we've failed to gather needed confirmations due to
            // request failures.

            tx.send(Err(QuorumNotEnough {
                cluster: eff_mem.membership().to_string(),
                got: granted,
            }
            .into()))
                .ok();
        };

        // TODO: do not spawn, manage read requests with a queue by RaftCore

        // False positive lint warning(`non-binding `let` on a future`): https://github.com/rust-lang/rust-clippy/issues/9932
        #[allow(clippy::let_underscore_future)]
        let _ = C::spawn(waiting_fu.instrument(tracing::debug_span!("spawn_is_leader_waiting")));
    }

    /// Submit change-membership by writing a Membership log entry.
    ///
    /// If `retain` is `true`, removed `voter` will becomes `learner`. Otherwise they will
    /// be just removed.
    ///
    /// Changing membership includes changing voters config or adding/removing learners:
    ///
    /// - To change voters config, it will build a new **joint** config. If it already a joint
    ///   config, it returns the final uniform config.
    /// - Adding a learner does not affect election, thus it does not need to enter joint consensus.
    ///   But it still has to wait for the previous membership to commit. Otherwise a second
    ///   proposed membership implies the previous one is committed.
    // ---
    // TODO: This limit can be removed if membership_state is replaced by a list of membership logs.
    //       Because allowing this requires the engine to be able to store more than 2
    //       membership logs. And it does not need to wait for the previous membership log to commit
    //       to propose the new membership log.
    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) fn change_membership(
        &mut self,
        changes: ChangeMembers<C>,
        retain: bool,
        tx: ProgressResponder<C, ClientWriteResult<C>>,
    ) {
        let res = self.engine.state.membership_state.change_handler().apply(changes, retain);
        let new_membership = match res {
            Ok(x) => x,
            Err(e) => {
                tx.on_complete(Err(ClientWriteError::ChangeMembershipError(e)));
                return;
            }
        };

        self.write_entries([EntryPayload::Membership(new_membership)], [Some(
            CoreResponder::Progress(tx),
        )]);
    }

    /// Ensure this node is a writable leader and return a leader handler.
    ///
    /// Returns `Err(ForwardToLeader)` if:
    /// - This node is not the leader
    /// - The leader is transferring leadership to another node
    fn ensure_writable_leader_handler(&mut self) -> Result<LeaderHandler<'_, C>, ForwardToLeader<C>> {
        let lh = self.engine.try_leader_handler()?;

        // If the leader is transferring leadership, forward writes to the new leader.
        if let Some(to) = lh.leader.get_transfer_to() {
            return Err(lh.state.new_forward_to_leader(to.clone()));
        }

        Ok(lh)
    }

    /// Write log entries to the cluster through raft protocol.
    ///
    /// I.e.: append the log entries to local store, forward them to a quorum(including the
    /// leader), waiting for them to be committed and applied.
    ///
    /// Returns the log IDs assigned to the entries, or `None` if the entries could not be
    /// written (e.g., this node is not the leader).
    ///
    /// The result of applying each entry to state machine is sent to its corresponding responder,
    /// if provided. The calling side may not receive a result if raft is shut down.
    ///
    /// The responder is either Responder type of [`RaftTypeConfig::Responder`]
    /// (application-defined) or [`ProgressResponder`] (general-purpose); the former is for
    /// application-defined entries like user data, the latter is for membership configuration
    /// changes.
    #[tracing::instrument(level = "debug", skip_all, fields(id = display(&self.id)))]
    pub fn write_entries<I, R>(&mut self, payloads: I, responders: R) -> Option<LeaderLogIds<C>>
    where
        I: IntoIterator<Item = EntryPayload<C>>,
        I::IntoIter: ExactSizeIterator,
        R: IntoIterator<Item = Option<CoreResponder<C>>>,
        R::IntoIter: ExactSizeIterator,
    {
        let payloads = payloads.into_iter();
        let responders = responders.into_iter();

        debug_assert_eq!(
            payloads.len(),
            responders.len(),
            "payloads and responders must have same length"
        );

        tracing::debug!("write {} entries", payloads.len());

        let mut lh = match self.ensure_writable_leader_handler() {
            Ok(lh) => lh,
            Err(forward_err) => {
                let err = ClientWriteError::ForwardToLeader(forward_err);
                for tx in responders.flatten() {
                    tx.on_complete(Err(err.clone()))
                }
                return None;
            }
        };

        // TODO: it should returns membership config error etc. currently this is done by the
        //       caller.
        let entry_count = payloads.len() as u64;
        let log_ids = lh.leader_append_entries(payloads)?;

        // Record write batch size to external metrics recorder
        if let Some(r) = &self.metrics_recorder {
            r.record_write_batch(entry_count);
        }

        for (log_id, resp_tx) in log_ids.clone().into_iter().zip(responders) {
            if let Some(tx) = resp_tx {
                let index = log_id.index();
                tracing::debug!("write entries: push tx to responders, log_id: {}", log_id);
                self.client_responders.push(index, tx);
            }
        }

        Some(log_ids)
    }

    /// Send a heartbeat message to every follower/learners.
    #[tracing::instrument(level = "debug", skip_all, fields(id = display(&self.id)))]
    pub(crate) fn send_heartbeat(&mut self, emitter: impl fmt::Display) -> bool {
        tracing::debug!("send heartbeat, now: {}", C::now().display());

        let Some(mut lh) = self.engine.try_leader_handler().ok() else {
            tracing::debug!(
                "{} failed to send heartbeat, not a Leader: now: {}",
                emitter,
                C::now().display()
            );
            return false;
        };

        if lh.leader.get_transfer_to().is_some() {
            tracing::debug!(
                "{} is transferring leadership, skip sending heartbeat: now: {}",
                emitter,
                C::now().display()
            );
            return false;
        }

        lh.send_heartbeat();

        // Record heartbeat to external metrics recorder
        if let Some(r) = &self.metrics_recorder {
            r.increment_heartbeat();
        }

        tracing::debug!("{} triggered sending heartbeat", emitter);
        true
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub fn flush_metrics(&mut self) {
        let io_state = self.engine.state.io_state();
        self.tx_progress.send_log_progress(io_state.log_progress.flushed().cloned());
        self.tx_progress.send_commit_progress(io_state.apply_progress.accepted().cloned());
        self.tx_progress.send_apply_progress(io_state.apply_progress.flushed().cloned());
        self.tx_progress.send_snapshot_progress(io_state.snapshot.flushed().cloned());

        let (replication, heartbeat) = if let Some(leader) = self.engine.leader.as_ref() {
            let replication_prog = &leader.progress;
            let replication = Some(replication_prog.collect_mapped(|item| item.to_matching_tuple()));

            let clock_prog = &leader.clock_progress;
            let heartbeat = Some(clock_prog.collect_mapped(|item| (item.id.clone(), item.val.map(SerdeInstant::new))));

            (replication, heartbeat)
        } else {
            (None, None)
        };

        self.report_metrics(replication, heartbeat);
    }

    /// Report a metrics payload on the current state of the Raft node.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn report_metrics(
        &mut self,
        replication: Option<ReplicationMetrics<C>>,
        heartbeat: Option<HeartbeatMetrics<C>>,
    ) {
        let last_quorum_acked = self.last_quorum_acked_time();
        let millis_since_quorum_ack = last_quorum_acked.map(|t| t.elapsed().as_millis() as u64);

        let st = &self.engine.state;

        let membership_config = st.membership_state.effective().stored_membership().clone();
        let current_leader = self.current_leader();

        // Get the last flushed vote, or use initial vote (term=0, node_id=self.id)
        // if no IO has been flushed yet (e.g., during startup).
        let vote = st
            .log_progress()
            .flushed()
            .map(|io_id| io_id.to_app_vote())
            .unwrap_or_else(|| VoteOf::<C>::new_with_default_term(self.id.clone()));

        #[allow(deprecated)]
        let m = RaftMetrics {
            running_state: Ok(()),
            id: self.id.clone(),

            // --- data ---
            current_term: st.vote_ref().term(),
            vote: vote.clone(),
            last_log_index: st.last_log_id().index(),
            last_applied: st.io_applied().cloned(),
            snapshot: st.io_snapshot_last_log_id().cloned(),
            purged: st.io_purged().cloned(),

            // --- cluster ---
            state: st.server_state,
            current_leader: current_leader.clone(),
            millis_since_quorum_ack,
            last_quorum_acked: last_quorum_acked.map(SerdeInstant::new),
            membership_config: membership_config.clone(),
            heartbeat: heartbeat.clone(),

            // --- replication ---
            replication: replication.clone(),
        };

        #[allow(deprecated)]
        let data_metrics = RaftDataMetrics {
            last_log: st.last_log_id().cloned(),
            last_applied: st.io_applied().cloned(),
            snapshot: st.io_snapshot_last_log_id().cloned(),
            purged: st.io_purged().cloned(),
            millis_since_quorum_ack,
            last_quorum_acked: last_quorum_acked.map(SerdeInstant::new),
            replication,
            heartbeat,
        };

        let server_metrics = RaftServerMetrics {
            id: self.id.clone(),
            vote: vote.clone(),
            state: st.server_state,
            current_leader,
            membership_config,
        };

        // Record to external metrics recorder
        if let Some(r) = &self.metrics_recorder {
            crate::metrics::forward_metrics(&m, r.as_ref());
        }

        // Start to send metrics
        // `RaftMetrics` is sent last, because `Wait` only examines `RaftMetrics`
        // but not `RaftDataMetrics` and `RaftServerMetrics`.
        // Thus if `RaftMetrics` change is perceived, the other two should have been updated.

        self.tx_data_metrics.send_if_modified(|metrix| {
            if data_metrics.ne(metrix) {
                *metrix = data_metrics.clone();
                return true;
            }
            false
        });

        self.tx_server_metrics.send_if_modified(|metrix| {
            if server_metrics.ne(metrix) {
                *metrix = server_metrics.clone();
                return true;
            }
            false
        });

        tracing::debug!("report metrics: {}", m);
        let res = self.tx_metrics.send(m);

        if let Err(err) = res {
            tracing::error!("failed to report metrics, error: {}, id: {}", err, &self.id);
        }
    }

    /// Handle the admin command `initialize`.
    ///
    /// It is allowed to initialize only when `last_log_id.is_none()` and `vote==(0,0)`.
    /// See: [Conditions for initialization][precondition]
    ///
    /// [precondition]: crate::docs::cluster_control::cluster_formation#preconditions-for-initialization
    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(crate) fn handle_initialize(
        &mut self,
        member_nodes: BTreeMap<C::NodeId, C::Node>,
        tx: ResultSender<C, (), InitializeError<C>>,
    ) {
        tracing::debug!("{}: member_nodes: {:?}", func_name!(), member_nodes);

        let membership = Membership::from(member_nodes);

        let res = self.engine.initialize(membership);

        let has_error = res.is_err();

        // If there is an error, respond at once.
        // Otherwise, wait for the initialization log to be applied to state machine.
        let condition = if has_error {
            None
        } else {
            // Wait for the initialization log to be flushed, not applied.
            //
            // Because committing a log entry requires a leader, the first leader may or may not be able to
            // established, for example, there are already other nodes in the cluster with more logs.
            //
            // Thus, initialization should never wait for to apply of the initialization log.
            //
            // When adding new learners after initialization, we should not wait for the log to be applied.
            // But this introduces an issue, if the client send a change membership at once after
            // initialization, it may receive a InProgress error:
            // `"InProgress": { "committed": null, "membership_log_id": { "leader_id": { "term": 0,
            //             "node_id": 0 }, "index": 0 } }`
            // TODO: change-membership should check leadership or wait for leader to establish?

            // Wait for the generated IO to be flushed before respond.
            let accepted = self.engine.state.io_state().log_progress.accepted().cloned();
            accepted.map(|io_id| Condition::IOFlushed { io_id })
        };
        self.engine.output.push_command(Command::Respond {
            when: condition,
            resp: Respond::new(res, tx),
        });

        if !has_error {
            // With the new config, start to elect to become leader
            self.engine.elect();
        }
    }

    /// Trigger a snapshot building(log compaction) job if there is no pending building job.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn trigger_snapshot(&mut self) {
        tracing::debug!("{}", func_name!());
        self.engine.snapshot_handler().trigger_snapshot();
    }

    /// Trigger routine actions that need to be checked after processing messages.
    ///
    /// This is called in the main event loop after processing messages and running engine commands.
    /// It performs routine checks and triggers corresponding actions:
    /// - Snapshot building based on `SnapshotPolicy`
    /// - Initiate replication if the replication stream is idle (for leader)
    ///
    /// Unlike tick-based triggers, this runs after every message batch, making it independent of
    /// the tick configuration and more responsive to state changes.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn trigger_routine_actions(&mut self) {
        // Check snapshot policy and trigger snapshot if needed
        if let Some(at) = self
            .config
            .snapshot_policy
            .should_snapshot(&self.engine.state, self.core_state.snapshot_tried_at.as_ref())
        {
            tracing::debug!("snapshot policy triggered at: {}", at);
            self.core_state.snapshot_tried_at = Some(at);
            self.trigger_snapshot();
        }

        // Keep replicating to a target if the replication stream to it is idle
        if let Ok(mut lh) = self.engine.try_leader_handler() {
            lh.replication_handler().initiate_replication();
        }

        // Broadcast I/O progress so replication tasks can read submitted logs.
        if let Some(submitted) = self.engine.state.log_progress().submitted().cloned() {
            self.io_submitted_tx.send_if_greater(submitted);
        }
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn current_leader(&self) -> Option<C::NodeId> {
        tracing::debug!(
            "get current_leader: self_id: {}, vote: {}",
            self.id,
            self.engine.state.vote_ref()
        );

        let vote = self.engine.state.vote_ref();

        if !vote.is_committed() {
            return None;
        }

        let id = vote.to_leader_id().node_id().clone();

        // TODO: `is_voter()` is slow, maybe cache `current_leader`,
        //       e.g., only update it when membership or vote changes
        if self.engine.state.membership_state.effective().is_voter(&id) {
            Some(id)
        } else {
            tracing::debug!("id={} is not a voter", id);
            None
        }
    }

    /// Retrieves the most recent timestamp that is acknowledged by a quorum.
    ///
    /// This function returns the latest known time at which the leader received acknowledgment
    /// from a quorum of followers, indicating its leadership is current and recognized.
    /// If the node is not a leader or no acknowledgment has been received, `None` is returned.
    fn last_quorum_acked_time(&mut self) -> Option<InstantOf<C>> {
        let leading = self.engine.leader.as_mut();
        leading.and_then(|l| l.last_quorum_acked_time())
    }

    pub(crate) fn get_leader_node(&self, leader_id: Option<C::NodeId>) -> Option<C::Node> {
        let leader_id = leader_id?;

        self.engine.state.membership_state.effective().get_node(&leader_id).cloned()
    }

    /// Apply log entries to the state machine, from the `first`(inclusive) to `last`(inclusive).
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn apply_to_state_machine(
        &mut self,
        first: LogIdOf<C>,
        last: LogIdOf<C>,
    ) -> Result<(), StorageError<C>> {
        tracing::debug!("{}: {}..={}", func_name!(), first, last);

        debug_assert!(
            first.index() <= last.index(),
            "first.index {} should <= last.index {}",
            first.index(),
            last.index()
        );

        #[cfg(debug_assertions)]
        if let Some(first_idx) = self.client_responders.first_index() {
            debug_assert!(
                first.index() <= first_idx,
                "first.index {} should <= client_resp_channels.first index {}",
                first.index(),
                first_idx,
            );
        }

        // Drain responders up to last.index
        let mut responders = self.client_responders.drain_upto(last.index());

        let entry_count = last.index() + 1 - first.index();
        self.runtime_stats.apply_batch.record(entry_count);

        // Record to external metrics recorder
        if let Some(r) = &self.metrics_recorder {
            r.record_apply_batch(entry_count);
        }

        // Call on_commit on each responder
        for (index, responder) in responders.iter_mut() {
            let log_id = self.engine.state.get_log_id(*index).unwrap();
            responder.on_commit(log_id);
        }

        let cmd = sm::Command::apply(first, last.clone(), responders);
        self.sm_handle.send(cmd).await.map_err(|e| StorageError::apply(last, C::err_from_string(e)))?;

        Ok(())
    }

    /// Spawn a new replication stream returning its replication state handle.
    #[tracing::instrument(level = "debug", skip(self))]
    #[allow(clippy::type_complexity)]
    pub(crate) async fn spawn_replication_stream(
        &mut self,
        leader_vote: CommittedVote<C>,
        prog: &TargetProgress<C>,
    ) -> ReplicationHandle<C> {
        let network = self.network_factory.new_client(prog.target.clone(), &prog.target_node).await;

        let (replicate_tx, replicate_rx) = C::watch_channel(Replicate::default());

        let event_watcher = self.new_event_watcher(replicate_rx);

        let (mut replication_handle, replication_context) = self.new_replication(leader_vote, prog, replicate_tx);

        let progress = replication_progress::ReplicationProgress {
            local_committed: self.engine.state.committed().cloned(),
            remote_matched: prog.progress.matching.clone(),
        };

        let join_handel = ReplicationCore::<C, NF, LS>::spawn(
            replication_context,
            progress,
            network,
            self.log_store.get_log_reader().await,
            event_watcher,
            tracing::span!(parent: &self.span, Level::DEBUG, "replication", id=display(&self.id), target=display(&prog.target)),
        );

        replication_handle.join_handle = Some(join_handel);

        replication_handle
    }

    fn new_replication(
        &self,
        leader_vote: CommittedVote<C>,
        prog: &TargetProgress<C>,
        replicate_tx: WatchSenderOf<C, Replicate<C>>,
    ) -> (ReplicationHandle<C>, ReplicationContext<C>) {
        let (cancel_tx, cancel_rx) = C::watch_channel(());

        let context = self.new_replication_context(leader_vote, prog, cancel_rx);

        let handle = ReplicationHandle::new(prog.progress.stream_id, replicate_tx, cancel_tx);

        (handle, context)
    }

    fn new_replication_context(
        &self,
        leader_vote: CommittedVote<C>,
        prog: &TargetProgress<C>,
        cancel_rx: WatchReceiverOf<C, ()>,
    ) -> ReplicationContext<C> {
        let id = self.id.clone();

        ReplicationContext {
            id,
            target: prog.target.clone(),
            leader_vote,
            stream_id: prog.progress.stream_id,
            config: self.config.clone(),
            tx_notify: self.tx_notification.clone(),
            cancel_rx,
            replicate_batch: self.shared_replicate_batch.clone(),
        }
    }

    fn new_event_watcher(&self, replicate_rx: WatchReceiverOf<C, Replicate<C>>) -> EventWatcher<C> {
        EventWatcher {
            replicate_rx,
            committed_rx: self.committed_tx.subscribe(),
            io_accepted_rx: self.io_accepted_tx.subscribe(),
            io_submitted_rx: self.io_submitted_tx.subscribe(),
        }
    }

    /// Run as many commands as possible.
    ///
    /// If there is a command that waits for a callback, just return and wait for
    /// next RaftMsg.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn run_engine_commands(&mut self) -> Result<(), StorageError<C>> {
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!("queued commands: start...");
            for c in self.engine.output.iter_commands() {
                tracing::debug!("queued commands: {:?}", c);
            }
            tracing::debug!("queued commands: end...");
        }

        self.send_satisfied_responds();

        loop {
            // Batch commands for better I/O performance (e.g., merge consecutive AppendEntries)
            self.engine.output.sched_commands(&self.config);

            let Some(cmd) = self.engine.output.pop_command() else {
                break;
            };

            let res = self.run_command(cmd).await?;

            let Some(cmd) = res else {
                // cmd executed. Process next
                continue;
            };

            // cmd is returned, means it can not be executed now, postpone it.

            tracing::debug!(
                "RAFT_stats id={:<2}    cmd: postpone command: {}, pending: {}",
                self.id,
                cmd,
                self.engine.output.len()
            );

            if self.engine.output.postpone_command(cmd).is_ok() {
                continue;
            }

            // cmd is put back to the front of the queue. quit the loop

            if tracing::enabled!(Level::DEBUG) {
                for c in self.engine.output.iter_commands().take(8) {
                    tracing::debug!("postponed, first 8 queued commands: {:?}", c);
                }
            }

            // A command must be postponed, but progress driven command may be ready to run.
            // Thus, we do not return, but find progress driven commands to run.
            break;
        }

        // Progress driven commands run at last because some command may generate progress changes.
        self.run_progress_driven_command().await?;

        Ok(())
    }

    /// Run all commands that are automatically generated by progress changes.
    async fn run_progress_driven_command(&mut self) -> Result<(), StorageError<C>> {
        while let Some(cmd) = self.engine.next_progress_driven_command() {
            tracing::debug!("RAFT_event id={:<2}    progress_driven cmd: {}", self.id, cmd);

            // IO progress generated command is always ready to run. no need to postpone.
            let res = self.run_command(cmd).await?;
            debug_assert!(res.is_none(), "progress driven command should always be executed");
        }

        Ok(())
    }

    /// Send responds whose waiting conditions are satisfied.
    ///
    /// Responds are queued when their waiting conditions (log flushed, applied, snapshot built)
    /// are not yet met. This method drains all responds whose conditions are now satisfied.
    pub(crate) fn send_satisfied_responds(&mut self) {
        let io_state = self.engine.state.io_state();

        tracing::debug!(
            "RAFT_stats id={:<2}    cmd: try send satisfied responds: log_io: {}, apply: {}, snapshot: {}",
            self.id,
            io_state.log_progress.flushed().display(),
            io_state.apply_progress.flushed().display(),
            io_state.snapshot.flushed().display(),
        );

        for (phase, respond) in self.engine.output.pending_responds.drain_satisfied(io_state) {
            tracing::debug!(
                "RAFT_stats id={:<2}    cmd: send respond waiting for {}: {}",
                self.id,
                phase,
                respond
            );
            respond.send();
        }
    }

    /// Run an event handling loop
    ///
    /// It always returns a [`Fatal`] error upon returning.
    #[tracing::instrument(level = "debug", skip_all, fields(id=display(&self.id)))]
    async fn runtime_loop(&mut self, mut rx_shutdown: OneshotReceiverOf<C, ()>) -> Result<Infallible, Fatal<C>> {
        // Ratio control the ratio of number of RaftMsg to process to number of Notification to process.
        let mut balancer = Balancer::new(10_000);

        loop {
            self.flush_metrics();

            tracing::debug!(
                "RAFT_stats id={:<2} log_io: {}",
                self.id,
                self.engine.state.log_progress()
            );

            // In each loop, it does not have to check rx_shutdown and flush metrics for every RaftMsg
            // processed.
            // In each loop, the first step is blocking waiting for any message from any channel.
            // Then if there is any message, process as many as possible to maximize throughput.

            // Check shutdown in each loop first so that a message flood in `tx_api` won't block shutting down.
            // `select!` without `biased` provides a random fairness.
            // We want to check shutdown prior to other channels.
            // See: https://docs.rs/tokio/latest/tokio/macro.select.html#fairness
            futures_util::select_biased! {
                _ = (&mut rx_shutdown).fuse() => {
                    tracing::info!("recv from rx_shutdown");
                    return Err(Fatal::Stopped);
                }

                notify_res = self.rx_notification.recv().fuse() => {
                    match notify_res {
                        Some(notify) => self.handle_notification(notify)?,
                        None => {
                            tracing::error!("all rx_notify senders are dropped");
                            return Err(Fatal::Stopped);
                        }
                    };
                }

                msg_res = self.rx_api.ensure_buffered().fuse() => {
                    msg_res?;
                }
            };

            self.run_engine_commands().await?;

            // There is a message waking up the loop, process channels one by one.

            let raft_msg_processed = self.process_raft_msg(balancer.raft_msg()).await?;
            let notify_processed = self.process_notification(balancer.notification()).await?;

            // If one of the channel consumed all its budget, re-balance the budget ratio.

            #[allow(clippy::collapsible_else_if)]
            if notify_processed == balancer.notification() {
                tracing::info!("there may be more Notification to process, increase Notification ratio");
                balancer.increase_notification();
            } else {
                if raft_msg_processed == balancer.raft_msg() {
                    tracing::info!("there may be more RaftMsg to process, increase RaftMsg ratio");
                    balancer.increase_raft_msg();
                }
            }

            // Trigger routine actions after processing all messages
            self.trigger_routine_actions();

            self.run_engine_commands().await?;
        }
    }

    /// Process RaftMsg as many as possible.
    ///
    /// It returns the number of processed message.
    /// If the input channel is closed, it returns `Fatal::Stopped`.
    async fn process_raft_msg(&mut self, at_most: u64) -> Result<u64, Fatal<C>> {
        self.runtime_stats.raft_msg_budget.record(at_most);

        let mut processed = 0u64;
        let mut total = 0u64;
        // Being 0 disabled batch msg processing.
        // TODO: make it configurable
        let run_command_threshold = 0;
        let mut last_log_index = 0;

        for _i in 0..at_most {
            let res = self.rx_api.try_recv()?;
            let Some(msg) = res else {
                break;
            };

            self.handle_api_msg(msg).await;
            processed += 1;
            total += 1;

            let index = self.engine.state.last_log_id().next_index();

            if index.saturating_sub(last_log_index) >= run_command_threshold {
                // After handling all the inputs, batch run all the commands for better performance
                self.runtime_stats.raft_msg_per_run.record(processed);
                self.runtime_stats.raft_msg_usage_permille.record(processed * 1000 / at_most);
                self.run_engine_commands().await?;

                last_log_index = index;
                processed = 0;
            }
        }

        // After handling all the inputs, batch run all the commands for better performance
        self.runtime_stats.raft_msg_per_run.record(processed);
        self.runtime_stats.raft_msg_usage_permille.record(processed * 1000 / at_most);
        self.run_engine_commands().await?;

        if total == at_most {
            tracing::debug!("at_most({}) reached, there are more queued RaftMsg to process", at_most);
        }

        Ok(total)
    }

    /// Process Notification as many as possible.
    ///
    /// It returns the number of processed notifications.
    /// If the input channel is closed, it returns `Fatal::Stopped`.
    async fn process_notification(&mut self, at_most: u64) -> Result<u64, Fatal<C>> {
        self.runtime_stats.notification_budget.record(at_most);

        let mut processed = 0u64;

        for _i in 0..at_most {
            let res = self.rx_notification.try_recv();
            let notify = match res {
                Ok(msg) => msg,
                Err(e) => match e {
                    TryRecvError::Empty => {
                        tracing::debug!("all Notification are processed, wait for more");
                        break;
                    }
                    TryRecvError::Disconnected => {
                        tracing::error!("rx_notify is disconnected, quit");
                        return Err(Fatal::Stopped);
                    }
                },
            };

            self.handle_notification(notify)?;
            processed += 1;

            // TODO: does run_engine_commands() run too frequently?
            //       to run many commands in one shot, it is possible to batch more commands to gain
            //       better performance.

            self.run_engine_commands().await?;
        }

        self.runtime_stats.notification_usage_permille.record(processed * 1000 / at_most);

        if processed == at_most {
            tracing::debug!(
                "at_most({}) reached, there are more queued Notification to process",
                at_most
            );
        }

        Ok(processed)
    }

    /// Spawn parallel vote requests to all cluster members.
    #[tracing::instrument(level = "trace", skip_all)]
    async fn spawn_parallel_vote_requests(&mut self, vote_req: &VoteRequest<C>) {
        let members = self.engine.state.membership_state.effective().voter_ids();

        let vote = vote_req.vote.clone();

        for target in members {
            if target == self.id {
                continue;
            }

            let req = vote_req.clone();

            // Safe unwrap(): target must be in membership
            let target_node = self.engine.state.membership_state.effective().get_node(&target).unwrap().clone();
            let mut client = self.network_factory.new_client(target.clone(), &target_node).await;

            let tx = self.tx_notification.clone();

            let ttl = Duration::from_millis(self.config.election_timeout_min);
            let id = self.id.clone();
            let option = RPCOption::new(ttl);

            let vote = vote.clone();

            // False positive lint warning(`non-binding `let` on a future`): https://github.com/rust-lang/rust-clippy/issues/9932
            #[allow(clippy::let_underscore_future)]
            let _ = C::spawn(
                {
                    let target = target.clone();
                    async move {
                        let tm_res = C::timeout(ttl, client.vote(req, option)).await;
                        let res = match tm_res {
                            Ok(res) => res,

                            Err(_timeout) => {
                                let timeout_err = Timeout::<C> {
                                    action: RPCTypes::Vote,
                                    id,
                                    target: target.clone(),
                                    timeout: ttl,
                                };
                                tracing::error!("timeout: {}", timeout_err);
                                return;
                            }
                        };

                        match res {
                            Ok(resp) => {
                                tx.send(Notification::VoteResponse {
                                    target,
                                    resp,
                                    candidate_vote: vote.into_non_committed(),
                                })
                                .await
                                .ok();
                            }
                            Err(err) => tracing::error!("while requesting vote, error: {}, target: {}", err, target),
                        }
                    }
                }
                .instrument(tracing::debug_span!(
                    parent: &Span::current(),
                    "send_vote_req",
                    target = display(&target)
                )),
            );
        }
    }

    /// Spawn parallel vote requests to all cluster members.
    #[tracing::instrument(level = "trace", skip_all)]
    async fn broadcast_transfer_leader(&mut self, req: TransferLeaderRequest<C>) {
        let voter_ids = self.engine.state.membership_state.effective().voter_ids();

        for target in voter_ids {
            if target == self.id {
                continue;
            }

            let r = req.clone();

            // Safe unwrap(): target must be in membership
            let target_node = self.engine.state.membership_state.effective().get_node(&target).unwrap().clone();
            let mut client = self.network_factory.new_client(target.clone(), &target_node).await;

            let ttl = Duration::from_millis(self.config.election_timeout_min);
            let option = RPCOption::new(ttl);

            let fut = {
                let target = target.clone();
                async move {
                    let tm_res = C::timeout(ttl, client.transfer_leader(r, option)).await;
                    let res = match tm_res {
                        Ok(res) => res,
                        Err(timeout) => {
                            tracing::error!("timeout sending transfer_leader: {}, target: {}", timeout, target);
                            return;
                        }
                    };

                    if let Err(e) = res {
                        tracing::error!("error sending transfer_leader: {}, target: {}", e, target);
                    } else {
                        tracing::info!("Done transfer_leader sent to {}", target);
                    }
                }
            };

            let span = tracing::debug_span!(
                parent: &Span::current(),
                "send_transfer_leader",
                target = display(&target)
            );

            // False positive lint warning(`non-binding `let` on a future`): https://github.com/rust-lang/rust-clippy/issues/9932
            #[allow(clippy::let_underscore_future)]
            let _ = C::spawn(fut.instrument(span));
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn handle_vote_request(&mut self, req: VoteRequest<C>, tx: VoteTx<C>) {
        tracing::info!("{}: req: {}", func_name!(), req);

        let resp = self.engine.handle_vote_req(req);

        // Record vote to external metrics recorder
        if let Some(r) = &self.metrics_recorder {
            r.increment_vote();
        }

        let condition = Some(Condition::IOFlushed {
            io_id: IOId::new(self.engine.state.vote_ref()),
        });
        self.engine.output.push_command(Command::Respond {
            when: condition,
            resp: Respond::new(resp, tx),
        });
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn handle_append_entries_request(&mut self, req: AppendEntriesRequest<C>, tx: AppendEntriesTx<C>) {
        tracing::debug!("{}: req: {}", func_name!(), req);

        self.engine.handle_append_entries(&req.vote, req.prev_log_id, req.entries, tx);

        // Record append entries to external metrics recorder
        if let Some(r) = &self.metrics_recorder {
            r.increment_append();
        }

        let committed = LogIOId::new(req.vote.to_committed(), req.leader_commit);
        self.engine.state.update_committed(committed);
    }

    // TODO: Make this method non-async. It does not need to run any async command in it.
    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = debug(self.engine.state.server_state), id=display(&self.id)
    ))]
    pub(crate) async fn handle_api_msg(&mut self, msg: RaftMsg<C>) {
        tracing::debug!("RAFT_event id={:<2}  input: {}", self.id, msg);

        self.runtime_stats.record_raft_msg(msg.name());

        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                self.handle_append_entries_request(rpc, tx);
            }
            RaftMsg::RequestVote { rpc, tx } => {
                let now = C::now();
                tracing::info!(
                    "received RaftMsg::RequestVote: {}, now: {}, vote_request: {}",
                    func_name!(),
                    now.display(),
                    rpc
                );

                self.handle_vote_request(rpc, tx);
            }
            RaftMsg::GetSnapshotReceiver { tx } => {
                self.engine.handle_begin_receiving_snapshot(tx);
            }
            RaftMsg::InstallSnapshot { vote, snapshot, tx } => {
                self.engine.handle_install_full_snapshot(vote, snapshot, tx);
            }
            RaftMsg::GetLinearizer { read_policy, tx } => {
                self.handle_ensure_linearizable_read(read_policy, tx).await;
            }
            RaftMsg::ClientWrite {
                payloads,
                responders,
                expected_leader,
            } => {
                // Check if expected leader matches current leader
                if let Some(expected) = expected_leader {
                    let vote = self.engine.state.vote_ref();

                    let committed_leader_id = vote.try_to_committed_leader_id();

                    if committed_leader_id.as_ref() != Some(&expected) {
                        // Leader has changed, return ForwardToLeader error to all responders
                        let forward_err = self.engine.state.forward_to_leader();
                        for r in responders.into_iter().flatten() {
                            let err = ClientWriteError::ForwardToLeader(forward_err.clone());
                            r.on_complete(Err(err));
                        }
                        return;
                    }
                }
                self.runtime_stats.write_batch.record(payloads.len() as u64);
                self.write_entries(payloads.into_iter(), responders.into_iter());
            }
            RaftMsg::Initialize { members, tx } => {
                tracing::info!("received RaftMsg::Initialize: {}, members: {:?}", func_name!(), members);

                self.handle_initialize(members, tx);
            }
            RaftMsg::ChangeMembership { changes, retain, tx } => {
                tracing::info!(
                    "received RaftMsg::ChangeMembership: {}, members: {:?}, retain: {:?}",
                    func_name!(),
                    changes,
                    retain
                );

                self.change_membership(changes, retain, tx);
            }
            RaftMsg::WithRaftState { req } => {
                req(&self.engine.state);
            }
            RaftMsg::HandleTransferLeader {
                from: current_leader_vote,
                to,
            } => {
                if self.engine.state.vote_ref() == &current_leader_vote {
                    tracing::info!("Transfer Leader from: {}, to {}", current_leader_vote, to);

                    self.engine.state.vote.disable_lease();
                    if self.id == to {
                        self.engine.elect();
                    }
                }
            }
            RaftMsg::ExternalCommand { cmd } => {
                tracing::info!("{}: received RaftMsg::ExternalCommand, cmd: {:?}", func_name!(), cmd);

                match cmd {
                    ExternalCommand::Elect => {
                        if self.engine.state.membership_state.effective().is_voter(&self.id) {
                            // TODO: reject if it is already a leader?
                            self.engine.elect();
                            tracing::debug!("ExternalCommand: triggered election");
                        } else {
                            // Node is switched to learner.
                        }
                    }
                    ExternalCommand::Heartbeat => {
                        self.send_heartbeat("ExternalCommand");
                    }
                    ExternalCommand::Snapshot => self.trigger_snapshot(),
                    ExternalCommand::GetSnapshot { tx } => {
                        let cmd = sm::Command::get_snapshot(tx);
                        let res = self.sm_handle.send(cmd).await;
                        if let Err(e) = res {
                            tracing::error!("error sending GetSnapshot to sm worker: {}", e);
                        }
                    }
                    ExternalCommand::PurgeLog { upto } => {
                        self.engine.trigger_purge_log(upto);
                    }
                    ExternalCommand::TriggerTransferLeader { to } => {
                        self.engine.trigger_transfer_leader(to);
                    }
                    ExternalCommand::AllowNextRevert { to, allow, tx } => {
                        //
                        let res = match self.engine.try_leader_handler() {
                            Ok(mut l) => {
                                let res = l.replication_handler().allow_next_revert(to, allow);
                                res.map_err(AllowNextRevertError::from)
                            }
                            Err(e) => {
                                tracing::warn!("AllowNextRevert: current node is not a Leader");
                                Err(AllowNextRevertError::from(e))
                            }
                        };
                        tx.send(res).ok();
                    }
                    ExternalCommand::StateMachineCommand { sm_cmd } => {
                        let res = self.sm_handle.send(sm_cmd).await;
                        if let Err(e) = res {
                            tracing::error!("error sending sm::Command to sm::Worker: {}", e);
                        }
                    }
                    ExternalCommand::SetMetricsRecorder { recorder } => {
                        tracing::info!("setting metrics recorder");
                        self.metrics_recorder = recorder;
                    }
                }
            }
            #[cfg(feature = "runtime-stats")]
            RaftMsg::GetRuntimeStats { tx } => {
                // Copy runtime_stats and sync the shared replicate_batch
                let mut stats = self.runtime_stats.clone();
                stats.replicate_batch = self.shared_replicate_batch.snapshot();
                tx.send(stats).ok();
            }
        };
    }

    #[tracing::instrument(level = "debug", skip_all, fields(state = debug(self.engine.state.server_state), id=display(&self.id)
    ))]
    pub(crate) fn handle_notification(&mut self, notify: Notification<C>) -> Result<(), Fatal<C>> {
        tracing::debug!("RAFT_event id={:<2} notify: {}", self.id, notify);

        self.runtime_stats.record_notification(notify.name());

        match notify {
            Notification::VoteResponse {
                target,
                resp,
                candidate_vote,
            } => {
                let now = C::now();

                tracing::info!(
                    "received Notification::VoteResponse: {}, now: {}, resp: {}",
                    func_name!(),
                    now.display(),
                    resp
                );

                #[allow(clippy::collapsible_if)]
                if self.engine.candidate.is_some() {
                    if self.does_candidate_vote_match(&candidate_vote, "VoteResponse") {
                        self.engine.handle_vote_resp(target, resp);
                    }
                }
            }

            Notification::HigherVote {
                target,
                higher,
                leader_vote,
            } => {
                tracing::info!(
                    "{}: received Notification::HigherVote, target: {}, higher_vote: {}, sending_vote: {}",
                    func_name!(),
                    target,
                    higher,
                    leader_vote
                );

                if self.does_leader_vote_match(&leader_vote, "HigherVote") {
                    // Rejected vote change is ok.
                    self.engine.vote_handler().update_vote(&higher).ok();
                }
            }

            Notification::Tick { i } => {
                // check every timer

                let now = C::now();
                tracing::debug!("received tick: {}, now: {}", i, now.display());

                self.handle_tick_election();

                // TODO: test: fixture: make isolated_nodes a single-way isolating.

                // Leader send heartbeat
                let heartbeat_at = self.engine.leader_ref().map(|l| l.next_heartbeat);
                if let Some(t) = heartbeat_at
                    && now >= t
                {
                    if self.runtime_config.enable_heartbeat.load(Ordering::Relaxed) {
                        self.send_heartbeat("tick");
                    }

                    // Install next heartbeat
                    if let Some(l) = self.engine.leader_mut() {
                        l.next_heartbeat = C::now() + Duration::from_millis(self.config.heartbeat_interval);
                    }
                }

                // When a membership that removes the leader is committed,
                // the leader continue to work for a short while before reverting to a learner.
                // This way, let the leader replicate the `membership-log-is-committed` message to
                // followers.
                // Otherwise, if the leader step down at once, the follower might have to
                // re-commit the membership log again, electing itself.
                //
                // ---
                //
                // Stepping down only when the response of the second change-membership is sent.
                // Otherwise the Sender to the caller will be dropped before sending back the
                // response.

                // TODO: temp solution: Manually wait until the second membership log being applied to state
                //       machine. Because the response is sent back to the caller after log is
                //       applied.
                //       ---
                //       A better way is to make leader step down a command that waits for the log to be applied.
                if self.engine.state.io_applied() >= self.engine.state.membership_state.effective().log_id().as_ref() {
                    self.engine.leader_step_down();
                }
            }

            Notification::StorageError { error } => {
                tracing::error!("RaftCore received Notification::StorageError: {}", error);
                return Err(Fatal::StorageError(error));
            }

            Notification::LocalIO { io_id } => {
                self.engine.state.log_progress_mut().try_flush(io_id.clone());

                match io_id {
                    IOId::Log(log_io_id) => {
                        // No need to check against membership change,
                        // because not like removing-then-adding a remote node,
                        // local log wont revert when membership changes.
                        #[allow(clippy::collapsible_if)]
                        if self.engine.leader.is_some() {
                            if self.does_leader_vote_match(&log_io_id.committed_vote, "LocalIO Notification") {
                                self.engine.replication_handler().update_local_progress(log_io_id.log_id);
                            }
                        }
                    }
                    IOId::Vote(_vote) => {
                        // nothing to do
                    }
                }
            }

            Notification::ReplicationProgress { progress, inflight_id } => {
                tracing::debug!("recv Notification::ReplicationProgress: progress: {}", progress);

                if let Some(mut rh) = self.engine.try_replication_handler() {
                    rh.update_progress(progress.target, progress.result, inflight_id);
                }
            }

            Notification::HeartbeatProgress {
                stream_id,
                sending_time,
                target,
            } => {
                if let Some(mut rh) = self.engine.try_replication_handler() {
                    rh.try_update_leader_clock(stream_id, target, sending_time);
                }
            }

            Notification::StateMachine { command_result } => {
                tracing::debug!("sm::StateMachine command result: {:?}", command_result);

                let res = command_result.result?;

                match res {
                    sm::Response::BuildSnapshotDone(meta) => {
                        tracing::info!(
                            "sm::StateMachine command done: BuildSnapshotDone: {}: {}",
                            meta.display(),
                            func_name!()
                        );

                        self.engine.on_building_snapshot_done(meta);
                    }
                    sm::Response::InstallSnapshot((log_io_id, meta)) => {
                        tracing::info!(
                            "sm::StateMachine command done: InstallSnapshot: {}, log_io_id: {}: {}",
                            meta.display(),
                            log_io_id,
                            func_name!()
                        );

                        self.engine.state.log_progress_mut().try_flush(IOId::Log(log_io_id));

                        if let Some(meta) = meta {
                            let st = self.engine.state.io_state_mut();
                            if let Some(last) = &meta.last_log_id {
                                st.apply_progress.try_flush(last.clone());
                                st.snapshot.try_flush(last.clone());
                            }
                        }
                    }
                    sm::Response::Apply(res) => {
                        self.engine.state.apply_progress_mut().try_flush(res.last_applied);
                    }
                }
            }
        };
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    fn handle_tick_election(&mut self) {
        let now = C::now();

        tracing::debug!("try to trigger election, now: {}", now.display());

        // TODO: leader lease should be extended. Or it has to examine if it is leader
        //       before electing.
        if self.engine.state.server_state == ServerState::Leader {
            tracing::debug!("skip election, already a leader");
            return;
        }

        if !self.engine.state.membership_state.effective().is_voter(&self.id) {
            tracing::debug!("skip election, not a voter");
            return;
        }

        if !self.runtime_config.enable_elect.load(Ordering::Relaxed) {
            tracing::debug!("skip election, election disabled");
            return;
        }

        if self.engine.state.membership_state.effective().voter_ids().count() == 1 {
            tracing::debug!("single voter, elect immediately");
        } else {
            tracing::debug!("multiple voters, check election timeout");

            let local_vote = &self.engine.state.vote;
            let timer_config = &self.engine.config.timer_config;

            let mut election_timeout = timer_config.election_timeout;

            if self.engine.is_there_greater_log() {
                election_timeout += timer_config.smaller_log_timeout;
            }

            tracing::debug!("local vote: {}, election_timeout: {:?}", local_vote, election_timeout,);

            if local_vote.is_expired(now, election_timeout) {
                tracing::info!("election timeout expired, triggering election");
            } else {
                tracing::debug!("election timeout not yet expired");
                return;
            }
        }

        // Every time elect, reset this flag.
        self.engine.reset_greater_log();

        tracing::info!("trigger election");
        self.engine.elect();
    }

    /// If a message is sent by a previous Candidate but is received by current Candidate,
    /// it is a stale message and should be just ignored.
    fn does_candidate_vote_match(&self, candidate_vote: &UncommittedVote<C>, msg: impl fmt::Display) -> bool {
        // If it finished voting, Candidate's vote is None.
        let Some(my_vote) = self.engine.candidate_ref().map(|x| x.vote_ref().clone()) else {
            tracing::warn!(
                "A message will be ignored because this node is no longer Candidate: \
                 msg sent by vote: {}; when ({})",
                candidate_vote,
                msg
            );
            return false;
        };

        if candidate_vote.leader_id() != my_vote.leader_id() {
            tracing::warn!(
                "A message will be ignored because vote changed: \
                msg sent by vote: {}; current my vote: {}; when ({})",
                candidate_vote,
                my_vote,
                msg
            );
            false
        } else {
            true
        }
    }

    /// If a message is sent by a previous Leader but is received by current Leader,
    /// it is a stale message and should be just ignored.
    fn does_leader_vote_match(&self, leader_vote: &CommittedVote<C>, msg: impl fmt::Display) -> bool {
        let Some(my_vote) = self.engine.leader.as_ref().map(|x| x.committed_vote.clone()) else {
            tracing::warn!(
                "A message will be ignored because this node is no longer Leader: \
                msg sent by vote: {}; when ({})",
                leader_vote,
                msg
            );
            return false;
        };

        if leader_vote != &my_vote {
            tracing::warn!(
                "A message will be ignored because vote changed: \
                msg sent by vote: {}; current my vote: {}; when ({})",
                leader_vote,
                my_vote,
                msg
            );
            false
        } else {
            true
        }
    }

    /// Broadcast heartbeat to all followers with per-follower matching log ids.
    ///
    /// This method validates the session and sends heartbeat events only if the current
    /// session matches the requested session (no leader change or membership change).
    fn broadcast_heartbeat(&mut self, session_id: ReplicationSessionId<C>) {
        // Lazy get the progress data for heartbeat. If the leader changes or replication
        // config changes, no need to send heartbeat.
        let Ok(lh) = self.engine.try_leader_handler() else {
            // No longer a leader
            return;
        };

        let committed_vote = lh.leader.committed_vote.clone();
        let membership_log_id = lh.state.membership_state.effective().log_id();
        let current_session_id = ReplicationSessionId::new(committed_vote, membership_log_id.clone());

        if current_session_id != session_id {
            // Session changed, skip heartbeat
            return;
        }

        let committed = lh.state.committed().cloned();
        let now = C::now();
        let events =
            lh.leader
                .progress
                .iter()
                .filter(|progress_entry| progress_entry.id != self.id)
                .map(|progress_entry| {
                    (progress_entry.id.clone(), HeartbeatEvent {
                        time: now,
                        matching: progress_entry.val.matching.clone(),
                        committed: committed.clone(),
                    })
                });

        self.heartbeat_handle.broadcast(events);
    }

    /// Creates a new replication context and its associated cancellation channel.
    ///
    /// Returns the context for the replication task and the sender half of the
    /// cancellation channel. Dropping the sender signals the task to stop.
    pub(crate) fn new_replication_task_context(
        &self,
        leader_vote: CommittedVote<C>,
        stream_id: StreamId,
        target: C::NodeId,
    ) -> (ReplicationContext<C>, WatchSenderOf<C, ()>) {
        let (cancel_tx, cancel_rx) = C::watch_channel(());
        let ctx = ReplicationContext {
            id: self.id.clone(),
            target,
            leader_vote,
            stream_id,
            config: self.config.clone(),
            tx_notify: self.tx_notification.clone(),
            cancel_rx,
            replicate_batch: self.shared_replicate_batch.clone(),
        };
        (ctx, cancel_tx)
    }

    async fn close_replication(target: &C::NodeId, mut s: ReplicationHandle<C>) {
        let Some(handle) = s.join_handle.take() else {
            return;
        };

        // Drop sender to notify the task to shutdown
        drop(s.replicate_tx);
        drop(s.cancel_tx);

        tracing::debug!("joining removed replication: {}", target);
        let _x = handle.await;
        tracing::info!("done joining removed replication: {}", target);
    }
}

impl<C, N, LS> RaftRuntime<C> for RaftCore<C, N, LS>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
{
    async fn run_command(&mut self, cmd: Command<C>) -> Result<Option<Command<C>>, StorageError<C>> {
        // tracing::debug!("RAFT_event id={:<2} trycmd: {}", self.id, cmd);

        let condition = cmd.condition();
        tracing::debug!("condition: {:?}", condition);

        if let Some(condition) = condition {
            if condition.is_met(&self.engine.state.io_state) {
                // continue run the command
            } else {
                tracing::debug!("{} is not yet met, postpone cmd: {}", condition, cmd);
                return Ok(Some(cmd));
            }
        }

        tracing::debug!("RAFT_event id={:<2}    cmd: {}", self.id, cmd);

        // Record command execution
        self.runtime_stats.record_command(cmd.name());

        match cmd {
            Command::UpdateIOProgress { io_id, .. } => {
                // Notify that I/O is about to be submitted.
                self.io_accepted_tx.send_if_greater(io_id.clone());

                self.engine.state.log_progress_mut().submit(io_id.clone());

                let notify = Notification::LocalIO { io_id: io_id.clone() };

                self.tx_notification.send(notify).await.ok();
            }
            Command::AppendEntries {
                committed_vote: vote,
                entries,
            } => {
                let last_log_id = entries.last().unwrap().log_id();
                tracing::debug!("AppendEntries: {}", entries.display_n(10));

                let entry_count = entries.len() as u64;

                // Record to internal histogram
                self.runtime_stats.append_batch.record(entry_count);

                // Record to external metrics recorder
                if let Some(r) = &self.metrics_recorder {
                    r.record_append_batch(entry_count);
                }

                let io_id = IOId::new_log_io(vote, Some(last_log_id));
                let callback = IOFlushed::new(io_id.clone(), self.tx_io_completed.clone());

                // Notify that I/O is about to be submitted.
                self.io_accepted_tx.send_if_greater(io_id.clone());

                // Mark this IO request as submitted,
                // other commands relying on it can then be processed.
                // For example,
                // `Replicate` command cannot run until this IO request is submitted(no need to be flushed),
                // because it needs to read the log entry from the log store.
                //
                // The `submit` state must be updated before calling `append()`,
                // because `append()` may call the callback before returning.
                self.engine.state.log_progress_mut().submit(io_id.clone());

                // Submit IO request, do not wait for the response.
                self.log_store.append(entries.into_iter(), callback).await.sto_write_logs()?;
            }
            Command::SaveVote { vote } => {
                let io_id = IOId::new(&vote);

                // Notify that vote I/O is about to be submitted.
                self.io_accepted_tx.send_if_greater(io_id.clone());

                self.engine.state.log_progress_mut().submit(io_id.clone());
                self.log_store.save_vote(&vote).await.sto_write_vote()?;

                self.tx_notification
                    .send(Notification::LocalIO {
                        io_id: IOId::new(&vote),
                    })
                    .await
                    .ok();

                // If a non-committed vote is saved,
                // there may be a candidate waiting for the response.
                if let VoteStatus::Pending(non_committed) = vote.clone().into_vote_status() {
                    self.tx_notification
                        .send(Notification::VoteResponse {
                            target: self.id.clone(),
                            // last_log_id is not used when sending VoteRequest to local node
                            resp: VoteResponse::new(vote, None, true),
                            candidate_vote: non_committed,
                        })
                        .await
                        .ok();
                }
            }
            Command::PurgeLog { upto } => {
                self.log_store.purge(upto.clone()).await.sto_write_logs()?;
                self.engine.state.io_state_mut().update_purged(Some(upto));
            }
            Command::TruncateLog { after } => {
                self.log_store.truncate_after(after.clone()).await.sto_write_logs()?;

                // Inform clients waiting for logs to be applied.
                let leader_id = self.current_leader();
                let leader_node = self.get_leader_node(leader_id.clone());

                for (log_index, tx) in self.client_responders.drain_from(after.next_index()) {
                    tx.on_complete(Err(ClientWriteError::ForwardToLeader(ForwardToLeader {
                        leader_id: leader_id.clone(),
                        leader_node: leader_node.clone(),
                    })));

                    tracing::debug!("sent ForwardToLeader for log_index: {}", log_index);
                }
            }
            Command::SendVote { vote_req } => {
                self.spawn_parallel_vote_requests(&vote_req).await;
            }
            Command::ReplicateCommitted { committed } => {
                self.committed_tx.send_if_greater(committed);
            }
            Command::BroadcastHeartbeat { session_id } => {
                self.broadcast_heartbeat(session_id);
            }
            Command::SaveCommittedAndApply {
                already_applied: already_committed,
                upto,
            } => {
                self.engine.state.apply_progress_mut().submit(upto.clone());

                self.log_store.save_committed(Some(upto.clone())).await.sto_write()?;

                let first = self.engine.state.get_log_id(already_committed.next_index()).unwrap();
                self.apply_to_state_machine(first, upto).await?;
            }
            Command::Replicate { req, target } => {
                let node = self.replications.get(&target).expect("replication to target node exists");
                node.replicate_tx.send(req).ok();
            }
            Command::ReplicateSnapshot {
                leader_vote,
                target,
                inflight_id,
            } => {
                let node = self.replications.get(&target).expect("replication to target node exists");

                let snapshot_reader = self.sm_handle.new_snapshot_reader();
                let stream_id = node.stream_id;
                let (replication_task_context, cancel_tx) =
                    self.new_replication_task_context(leader_vote, stream_id, target.clone());

                let target_node = self.engine.state.membership_state.effective().get_node(&target).unwrap();
                let snapshot_network = self.network_factory.new_client(target.clone(), target_node).await;

                let handle = SnapshotTransmitter::<C, N>::spawn(
                    replication_task_context,
                    snapshot_network,
                    snapshot_reader,
                    inflight_id,
                    cancel_tx,
                );

                let node = self.replications.get_mut(&target).expect("replication to target node exists");
                // TODO: it is not cleaned when snapshot transmission is done.
                node.snapshot_transmit_handle = Some(handle);
            }
            Command::BroadcastTransferLeader { req } => self.broadcast_transfer_leader(req).await,

            Command::CloseReplicationStreams => {
                self.heartbeat_handle.close_workers();

                let left = std::mem::take(&mut self.replications);
                for (target, s) in left {
                    Self::close_replication(&target, s).await;
                }
            }
            Command::RebuildReplicationStreams {
                leader_vote,
                targets,
                close_old_streams,
            } => {
                self.heartbeat_handle
                    .spawn_workers(
                        leader_vote.clone(),
                        &mut self.network_factory,
                        &self.tx_notification,
                        &targets,
                        close_old_streams,
                    )
                    .await;

                let mut new_replications = BTreeMap::new();

                for prog in targets.iter() {
                    let removed = self.replications.remove(&prog.target);

                    let handle = if let Some(removed) = removed {
                        if close_old_streams {
                            Self::close_replication(&prog.target, removed).await;
                            None
                        } else {
                            Some(removed)
                        }
                    } else {
                        None
                    };

                    let handle = if let Some(handle) = handle {
                        handle
                    } else {
                        self.spawn_replication_stream(leader_vote.clone(), prog).await
                    };

                    new_replications.insert(prog.target.clone(), handle);
                }

                tracing::debug!("removing unused replications");

                let left = std::mem::replace(&mut self.replications, new_replications);

                for (target, s) in left {
                    Self::close_replication(&target, s).await;
                }
            }
            Command::StateMachine { command } => {
                let io_id = command.get_log_progress();

                if let Some(io_id) = io_id {
                    self.engine.state.log_progress_mut().submit(io_id);
                }

                // If this command update the last-applied log id, mark it as submitted(to state machine).
                if let Some(log_id) = command.get_apply_progress() {
                    self.engine.state.apply_progress_mut().submit(log_id);
                }

                if let Some(log_id) = command.get_snapshot_progress() {
                    self.engine.state.snapshot_progress_mut().submit(log_id);
                }

                // Just forward a state machine command to the worker.
                self.sm_handle
                    .send(command)
                    .await
                    .map_err(|_e| StorageError::write_state_machine(C::err_from_string("cannot send to sm::Worker")))?;
            }
            Command::Respond { resp: send, .. } => {
                send.send();
            }
        }

        Ok(None)
    }
}
