use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyerror::AnyError;
use futures::FutureExt;
use futures::StreamExt;
use futures::TryFutureExt;
use futures::stream::FuturesUnordered;
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
use crate::async_runtime::MpscUnboundedSender;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::TryRecvError;
use crate::async_runtime::watch::WatchSender;
use crate::config::Config;
use crate::config::RuntimeConfig;
use crate::core::ClientResponderQueue;
use crate::core::ServerState;
use crate::core::balancer::Balancer;
use crate::core::core_state::CoreState;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::heartbeat::handle::HeartbeatWorkersHandle;
use crate::core::io_flush_tracking::IoProgressSender;
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
use crate::display_ext::DisplaySliceExt;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::Engine;
use crate::engine::ReplicationProgress;
use crate::engine::Respond;
use crate::entry::RaftEntry;
use crate::error::AllowNextRevertError;
use crate::error::ClientWriteError;
use crate::error::Fatal;
use crate::error::ForwardToLeader;
use crate::error::Infallible;
use crate::error::InitializeError;
use crate::error::QuorumNotEnough;
use crate::error::RPCError;
use crate::error::StorageIOResult;
use crate::error::Timeout;
use crate::impls::ProgressResponder;
use crate::log_id::option_raft_log_id_ext::OptionRaftLogIdExt;
use crate::metrics::HeartbeatMetrics;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftMetrics;
use crate::metrics::RaftServerMetrics;
use crate::metrics::ReplicationMetrics;
use crate::metrics::SerdeInstant;
use crate::network::RPCOption;
use crate::network::RPCTypes;
use crate::network::RaftNetworkFactory;
use crate::network::v2::RaftNetworkV2;
use crate::progress::Progress;
use crate::progress::entry::ProgressEntry;
use crate::quorum::QuorumSet;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::ClientWriteResult;
use crate::raft::ReadPolicy;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::linearizable_read::Linearizer;
use crate::raft::message::TransferLeaderRequest;
use crate::raft::responder::Responder;
use crate::raft::responder::core_responder::CoreResponder;
use crate::raft_state::LogStateReader;
use crate::raft_state::RuntimeStats;
use crate::raft_state::io_state::io_id::IOId;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::replication::ReplicationCore;
use crate::replication::ReplicationHandle;
use crate::replication::ReplicationSessionId;
use crate::replication::replication_context::ReplicationContext;
use crate::replication::request::Replicate;
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
use crate::type_config::alias::WatchSenderOf;
use crate::type_config::alias::WriteResponderOf;
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
    pub(crate) rx_api: MpscReceiverOf<C, RaftMsg<C>>,

    /// A Sender to send callback by other components to [`RaftCore`], when an action is finished,
    /// such as flushing log to disk, or applying log entries to state machine.
    pub(crate) tx_notification: MpscSenderOf<C, Notification<C>>,

    /// A Receiver to receive callback from other components.
    pub(crate) rx_notification: MpscReceiverOf<C, Notification<C>>,

    /// A Watch channel sender for IO completion notifications from storage callbacks.
    /// This is used by IOFlushed callbacks to report IO completion in a synchronous manner.
    pub(crate) tx_io_completed: WatchSenderOf<C, Result<IOId<C>, StorageError<C>>>,

    pub(crate) tx_metrics: WatchSenderOf<C, RaftMetrics<C>>,
    pub(crate) tx_data_metrics: WatchSenderOf<C, RaftDataMetrics<C>>,
    pub(crate) tx_server_metrics: WatchSenderOf<C, RaftServerMetrics<C>>,
    pub(crate) tx_progress: IoProgressSender<C>,

    pub(crate) runtime_stats: RuntimeStats,

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
                tracing::error!(error = display(&err), "quit RaftCore::main on error");
            }
        }

        tracing::debug!("update the metrics for shutdown");
        {
            let mut curr = self.tx_metrics.borrow_watched().clone();
            curr.state = ServerState::Shutdown;
            curr.running_state = Err(err.clone());

            let _ = self.tx_metrics.send(curr);
        }

        tracing::info!("RaftCore shutdown complete");

        Err(err)
    }

    #[tracing::instrument(level="trace", skip_all, fields(id=display(&self.id), cluster=%self.config.cluster_name))]
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
            let l = self.engine.leader_handler();
            let lh = match l {
                Ok(leading_handler) => leading_handler,
                Err(forward) => {
                    let _ = tx.send(Err(forward.into()));
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
                let _ = tx.send(Ok(resp));
                return;
            }
            tracing::debug!("{}: lease expired when do lease read", self.id);
            // we may no longer leader so error out early
            let err = ForwardToLeader::empty();
            let _ = tx.send(Err(err.into()));
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
            let _ = tx.send(Ok(resp));
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
                    let outer_res = C::timeout(ttl, client.append_entries(rpc, option)).await;
                    match outer_res {
                        Ok(append_res) => match append_res {
                            Ok(x) => Ok((target, x)),
                            Err(err) => Err((target, err)),
                        },
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
                let (target, append_res) = match res {
                    Ok(Ok(res)) => res,
                    Ok(Err((target, err))) => {
                        tracing::error!(target=display(target), error=%err, "timeout while confirming leadership for read request");
                        continue;
                    }
                    Err((target, err)) => {
                        tracing::error!(target = display(target), "fail to join task: {}", err);
                        continue;
                    }
                };

                // If we receive a response with a greater vote, then revert to follower and abort this
                // request.
                if let AppendEntriesResponse::HigherVote(vote) = append_res {
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
                        tracing::error!("fail to send HigherVote to RaftCore");
                    }

                    // we are no longer leader so error out early
                    let err = ForwardToLeader::empty();
                    let _ = tx.send(Err(err.into()));
                    return;
                }

                granted.insert(target);

                if eff_mem.is_quorum(granted.iter()) {
                    let _ = tx.send(Ok(resp));
                    return;
                }
            }

            // If we've hit this location, then we've failed to gather needed confirmations due to
            // request failures.

            let _ = tx.send(Err(QuorumNotEnough {
                cluster: eff_mem.membership().to_string(),
                got: granted,
            }
            .into()));
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

        let ent = C::Entry::new_membership(LogIdOf::<C>::default(), new_membership);
        self.write_entry(ent, Some(CoreResponder::Progress(tx)));
    }

    /// Write a log entry to the cluster through raft protocol.
    ///
    /// I.e.: append the log entry to local store, forward it to a quorum(including the leader),
    /// waiting for it to be committed and applied.
    ///
    /// The result of applying it to state machine is sent to `resp_tx`, if it is not `None`.
    /// The calling side may not receive a result from `resp_tx`, if raft is shut down.
    ///
    /// The responder `resp_tx` is either Responder type of
    /// [`RaftTypeConfig::Responder`] (application-defined) or [`ProgressResponder`]
    /// (general-purpose); the former is for application-defined entries like user data, the
    /// latter is for membership configuration changes.
    #[tracing::instrument(level = "debug", skip_all, fields(id = display(&self.id)))]
    pub fn write_entry(&mut self, entry: C::Entry, resp_tx: Option<CoreResponder<C>>) {
        tracing::debug!(payload = display(&entry), "write_entry");

        let Some((mut lh, tx)) = self.engine.get_leader_handler_or_reject(resp_tx) else {
            return;
        };

        // If the leader is transferring leadership, forward writes to the new leader.
        if let Some(to) = lh.leader.get_transfer_to() {
            if let Some(tx) = tx {
                let err = lh.state.new_forward_to_leader(to.clone());
                tx.on_complete(Err(ClientWriteError::ForwardToLeader(err)));
            }
            return;
        }

        let entries = vec![entry];
        // TODO: it should returns membership config error etc. currently this is done by the
        //       caller.
        lh.leader_append_entries(entries);
        let index = lh.state.last_log_id().unwrap().index();

        // Install callback channels.
        if let Some(tx) = tx {
            self.client_responders.push(index, tx);
        }
    }

    /// Send a heartbeat message to every follower/learners.
    #[tracing::instrument(level = "debug", skip_all, fields(id = display(&self.id)))]
    pub(crate) fn send_heartbeat(&mut self, emitter: impl fmt::Display) -> bool {
        tracing::debug!(now = display(C::now().display()), "send_heartbeat");

        let Some((mut lh, _)) = self.engine.get_leader_handler_or_reject(None::<WriteResponderOf<C>>) else {
            tracing::debug!(
                now = display(C::now().display()),
                "{} failed to send heartbeat, not a Leader",
                emitter
            );
            return false;
        };

        if lh.leader.get_transfer_to().is_some() {
            tracing::debug!(
                now = display(C::now().display()),
                "{} is transferring leadership, skip sending heartbeat",
                emitter
            );
            return false;
        }

        lh.send_heartbeat();

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

        tracing::debug!("report_metrics: {}", m);
        let res = self.tx_metrics.send(m);

        if let Err(err) = res {
            tracing::error!(error=%err, id=display(&self.id), "error reporting metrics");
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
        tracing::debug!(member_nodes = debug(&member_nodes), "{}", func_name!());

        let membership = Membership::from(member_nodes);

        let entry = C::Entry::new_membership(LogIdOf::<C>::default(), membership);
        let res = self.engine.initialize(entry);

        // If there is an error, respond at once.
        // Otherwise, wait for the initialization log to be applied to state machine.
        let condition = if res.is_err() {
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

        // With the new config, start to elect to become leader
        self.engine.elect();
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
        if let Ok(mut lh) = self.engine.leader_handler() {
            lh.replication_handler().initiate_replication();
        }
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn current_leader(&self) -> Option<C::NodeId> {
        tracing::debug!(
            self_id = display(&self.id),
            vote = display(self.engine.state.vote_ref()),
            "get current_leader"
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

        // Call on_commit on each responder
        for (index, responder) in responders.iter_mut() {
            let log_id = self.engine.state.get_log_id(*index).unwrap();
            responder.on_commit(log_id);
        }

        let cmd = sm::Command::apply(first, last.clone(), responders);
        self.sm_handle.send(cmd).map_err(|e| StorageError::apply(last, AnyError::error(e)))?;

        Ok(())
    }

    /// Spawn a new replication stream returning its replication state handle.
    #[tracing::instrument(level = "debug", skip(self))]
    #[allow(clippy::type_complexity)]
    pub(crate) async fn spawn_replication_stream(
        &mut self,
        target: C::NodeId,
        progress_entry: ProgressEntry<C>,
    ) -> ReplicationHandle<C> {
        // Safe unwrap(): target must be in membership
        let target_node = self.engine.state.membership_state.effective().get_node(&target).unwrap();

        let membership_log_id = self.engine.state.membership_state.effective().log_id();
        let network = self.network_factory.new_client(target.clone(), target_node).await;

        let leader = self.engine.leader.as_ref().unwrap();

        let session_id = ReplicationSessionId::new(leader.committed_vote.clone(), membership_log_id.clone());

        ReplicationCore::<C, NF, LS>::spawn(
            target.clone(),
            session_id,
            self.config.clone(),
            self.engine.state.committed().cloned(),
            progress_entry.matching.clone(),
            network,
            self.log_store.get_log_reader().await,
            self.tx_notification.clone(),
            tracing::span!(parent: &self.span, Level::DEBUG, "replication", id=display(&self.id), target=display(&target)),
        )
    }

    /// Remove all replication.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn remove_all_replication(&mut self) {
        tracing::info!("remove all replication");

        self.heartbeat_handle.shutdown();

        let nodes = std::mem::take(&mut self.replications);

        tracing::debug!(
            targets = debug(nodes.iter().map(|x| x.0.clone()).collect::<Vec<_>>()),
            "remove all targets from replication_metrics"
        );

        for (target, s) in nodes {
            let handle = s.join_handle;

            // Drop sender to notify the task to shutdown
            drop(s.tx_repl);

            tracing::debug!("joining removed replication: {}", target);
            let _x = handle.await;
            tracing::info!("Done joining removed replication : {}", target);
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

        while let Some(cmd) = self.engine.output.pop_command() {
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
    #[tracing::instrument(level="debug", skip_all, fields(id=display(&self.id)))]
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
            futures::select_biased! {
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

                msg_res = self.rx_api.recv().fuse() => {
                    match msg_res {
                        Some(msg) => self.handle_api_msg(msg).await,
                        None => {
                            tracing::info!("all rx_api senders are dropped");
                            return Err(Fatal::Stopped);
                        }
                    };
                }
            }

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
        for i in 0..at_most {
            let res = self.rx_api.try_recv();
            let msg = match res {
                Ok(msg) => msg,
                Err(e) => match e {
                    TryRecvError::Empty => {
                        tracing::debug!("all RaftMsg are processed, wait for more");
                        return Ok(i + 1);
                    }
                    TryRecvError::Disconnected => {
                        tracing::debug!("rx_api is disconnected, quit");
                        return Err(Fatal::Stopped);
                    }
                },
            };

            self.handle_api_msg(msg).await;

            // TODO: does run_engine_commands() run too frequently?
            //       to run many commands in one shot, it is possible to batch more commands to gain
            //       better performance.

            self.run_engine_commands().await?;
        }

        tracing::debug!("at_most({}) reached, there are more queued RaftMsg to process", at_most);

        Ok(at_most)
    }

    /// Process Notification as many as possible.
    ///
    /// It returns the number of processed notifications.
    /// If the input channel is closed, it returns `Fatal::Stopped`.
    async fn process_notification(&mut self, at_most: u64) -> Result<u64, Fatal<C>> {
        for i in 0..at_most {
            let res = self.rx_notification.try_recv();
            let notify = match res {
                Ok(msg) => msg,
                Err(e) => match e {
                    TryRecvError::Empty => {
                        tracing::debug!("all Notification are processed, wait for more");
                        return Ok(i + 1);
                    }
                    TryRecvError::Disconnected => {
                        tracing::error!("rx_notify is disconnected, quit");
                        return Err(Fatal::Stopped);
                    }
                },
            };

            self.handle_notification(notify)?;

            // TODO: does run_engine_commands() run too frequently?
            //       to run many commands in one shot, it is possible to batch more commands to gain
            //       better performance.

            self.run_engine_commands().await?;
        }

        tracing::debug!(
            "at_most({}) reached, there are more queued Notification to process",
            at_most
        );

        Ok(at_most)
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
                                tracing::error!({error = %timeout_err}, "timeout");
                                return;
                            }
                        };

                        match res {
                            Ok(resp) => {
                                let _ = tx
                                    .send(Notification::VoteResponse {
                                        target,
                                        resp,
                                        candidate_vote: vote.into_non_committed(),
                                    })
                                    .await;
                            }
                            Err(err) => tracing::error!({error=%err, target=display(&target)}, "while requesting vote"),
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
                            tracing::error!({error = display(timeout), target = display(&target)}, "timeout sending transfer_leader");
                            return;
                        }
                    };

                    if let Err(e) = res {
                        tracing::error!({error = display(e), target = display(&target)}, "error sending transfer_leader");
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
        tracing::info!(req = display(&req), func = func_name!());

        let resp = self.engine.handle_vote_req(req);
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
        // TODO: test this function when `is_ok` is removed, it should be able to get the right conclusion
        // it self.
        tracing::debug!(req = display(&req), func = func_name!());

        let is_ok = self.engine.handle_append_entries(&req.vote, req.prev_log_id, req.entries, tx);

        if is_ok {
            let committed = LogIOId::new(req.vote.to_committed(), req.leader_commit);
            self.engine.state.update_committed(committed);
        }
    }

    // TODO: Make this method non-async. It does not need to run any async command in it.
    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = debug(self.engine.state.server_state), id=display(&self.id)))]
    pub(crate) async fn handle_api_msg(&mut self, msg: RaftMsg<C>) {
        tracing::debug!("RAFT_event id={:<2}  input: {}", self.id, msg);

        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                self.handle_append_entries_request(rpc, tx);
            }
            RaftMsg::RequestVote { rpc, tx } => {
                let now = C::now();
                tracing::info!(
                    now = display(now.display()),
                    vote_request = display(&rpc),
                    "received RaftMsg::RequestVote: {}",
                    func_name!()
                );

                self.handle_vote_request(rpc, tx);
            }
            RaftMsg::BeginReceivingSnapshot { tx } => {
                self.engine.handle_begin_receiving_snapshot(tx);
            }
            RaftMsg::InstallFullSnapshot { vote, snapshot, tx } => {
                self.engine.handle_install_full_snapshot(vote, snapshot, tx);
            }
            RaftMsg::EnsureLinearizableRead { read_policy, tx } => {
                self.handle_ensure_linearizable_read(read_policy, tx).await;
            }
            RaftMsg::ClientWriteRequest {
                app_data,
                responder,
                expected_leader,
            } => {
                // Check if expected leader matches current leader
                if let Some(expected) = expected_leader {
                    let vote = self.engine.state.vote_ref();

                    let committed_leader_id = vote.try_to_committed_leader_id();

                    if committed_leader_id.as_ref() != Some(&expected) {
                        // Leader has changed, return ForwardToLeader error
                        if let Some(responder) = responder {
                            let forward_err = self.engine.state.forward_to_leader();
                            let client_write_err = ClientWriteError::ForwardToLeader(forward_err);
                            responder.on_complete(Err(client_write_err));
                        }
                        return;
                    }
                }
                self.write_entry(C::Entry::new_normal(LogIdOf::<C>::default(), app_data), responder);
            }
            RaftMsg::Initialize { members, tx } => {
                tracing::info!(
                    members = debug(&members),
                    "received RaftMsg::Initialize: {}",
                    func_name!()
                );

                self.handle_initialize(members, tx);
            }
            RaftMsg::ChangeMembership { changes, retain, tx } => {
                tracing::info!(
                    members = debug(&changes),
                    retain = debug(&retain),
                    "received RaftMsg::ChangeMembership: {}",
                    func_name!()
                );

                self.change_membership(changes, retain, tx);
            }
            RaftMsg::ExternalCoreRequest { req } => {
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
                tracing::info!(cmd = debug(&cmd), "received RaftMsg::ExternalCommand: {}", func_name!());

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
                        let res = self.sm_handle.send(cmd);
                        if let Err(e) = res {
                            tracing::error!(error = display(e), "error sending GetSnapshot to sm worker");
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
                        let res = match self.engine.leader_handler() {
                            Ok(mut l) => {
                                let res = l.replication_handler().allow_next_revert(to, allow);
                                res.map_err(AllowNextRevertError::from)
                            }
                            Err(e) => {
                                tracing::warn!("AllowNextRevert: current node is not a Leader");
                                Err(AllowNextRevertError::from(e))
                            }
                        };
                        let _ = tx.send(res);
                    }
                    ExternalCommand::StateMachineCommand { sm_cmd } => {
                        let res = self.sm_handle.send(sm_cmd);
                        if let Err(e) = res {
                            tracing::error!(error = display(e), "error sending sm::Command to sm::Worker");
                        }
                    }
                }
            }
        };
    }

    #[tracing::instrument(level = "debug", skip_all, fields(state = debug(self.engine.state.server_state), id=display(&self.id)))]
    pub(crate) fn handle_notification(&mut self, notify: Notification<C>) -> Result<(), Fatal<C>> {
        tracing::debug!("RAFT_event id={:<2} notify: {}", self.id, notify);

        match notify {
            Notification::VoteResponse {
                target,
                resp,
                candidate_vote,
            } => {
                let now = C::now();

                tracing::info!(
                    now = display(now.display()),
                    resp = display(&resp),
                    "received Notification::VoteResponse: {}",
                    func_name!()
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
                    target = display(target),
                    higher_vote = display(&higher),
                    sending_vote = display(&leader_vote),
                    "received Notification::HigherVote: {}",
                    func_name!()
                );

                if self.does_leader_vote_match(&leader_vote, "HigherVote") {
                    // Rejected vote change is ok.
                    let _ = self.engine.vote_handler().update_vote(&higher);
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
                // If vote or membership changes, ignore the message.
                // There is chance delayed message reports a wrong state.
                if self.does_replication_session_match(&progress.session_id, "ReplicationProgress") {
                    tracing::debug!(progress = display(&progress), "recv Notification::ReplicationProgress");

                    // replication_handler() won't panic because:
                    // The leader is still valid because progress.session_id.leader_vote does not change.
                    self.engine.replication_handler().update_progress(progress.target, progress.result, inflight_id);
                }
            }

            Notification::HeartbeatProgress {
                session_id,
                sending_time,
                target,
            } => {
                if self.does_replication_session_match(&session_id, "HeartbeatProgress") {
                    tracing::debug!(
                        session_id = display(&session_id),
                        target = display(&target),
                        sending_time = display(sending_time.display()),
                        "HeartbeatProgress"
                    );
                    // replication_handler() won't panic because:
                    // The leader is still valid because progress.session_id.leader_vote does not change.
                    self.engine.replication_handler().update_leader_clock(target, sending_time);
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

        tracing::debug!("try to trigger election by tick, now: {}", now.display());

        // TODO: leader lease should be extended. Or it has to examine if it is leader
        //       before electing.
        if self.engine.state.server_state == ServerState::Leader {
            tracing::debug!("already a leader, do not elect again");
            return;
        }

        if !self.engine.state.membership_state.effective().is_voter(&self.id) {
            tracing::debug!("this node is not a voter");
            return;
        }

        if !self.runtime_config.enable_elect.load(Ordering::Relaxed) {
            tracing::debug!("election is disabled");
            return;
        }

        if self.engine.state.membership_state.effective().voter_ids().count() == 1 {
            tracing::debug!("this is the only voter, do election at once");
        } else {
            tracing::debug!("there are multiple voter, check election timeout");

            let local_vote = &self.engine.state.vote;
            let timer_config = &self.engine.config.timer_config;

            let mut election_timeout = timer_config.election_timeout;

            if self.engine.is_there_greater_log() {
                election_timeout += timer_config.smaller_log_timeout;
            }

            tracing::debug!("local vote: {}, election_timeout: {:?}", local_vote, election_timeout,);

            if local_vote.is_expired(now, election_timeout) {
                tracing::info!("election timeout passed, about to elect");
            } else {
                tracing::debug!("election timeout has not yet passed",);
                return;
            }
        }

        // Every time elect, reset this flag.
        self.engine.reset_greater_log();

        tracing::info!("do trigger election");
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

    /// If a message is sent by a previous replication session but is received by current server
    /// state, it is a stale message and should be just ignored.
    fn does_replication_session_match(
        &self,
        session_id: &ReplicationSessionId<C>,
        msg: impl fmt::Display + Copy,
    ) -> bool {
        if !self.does_leader_vote_match(&session_id.committed_vote(), msg) {
            return false;
        }

        if &session_id.membership_log_id != self.engine.state.membership_state.effective().log_id() {
            tracing::warn!(
                "membership_log_id changed: msg sent by: {}; curr: {}; ignore when ({})",
                session_id.membership_log_id.display(),
                self.engine.state.membership_state.effective().log_id().display(),
                msg
            );
            return false;
        }
        true
    }

    /// Broadcast heartbeat to all followers with per-follower matching log ids.
    ///
    /// This method validates the session and sends heartbeat events only if the current
    /// session matches the requested session (no leader change or membership change).
    fn broadcast_heartbeat(&mut self, session_id: ReplicationSessionId<C>) {
        // Lazy get the progress data for heartbeat. If the leader changes or replication
        // config changes, no need to send heartbeat.
        let Ok(lh) = self.engine.leader_handler() else {
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
                        session_id: session_id.clone(),
                        matching: progress_entry.val.matching.clone(),
                        committed: committed.clone(),
                    })
                });

        self.heartbeat_handle.broadcast(events);
    }

    pub(crate) fn new_replication_task_state(
        &self,
        session_id: ReplicationSessionId<C>,
        target: C::NodeId,
    ) -> ReplicationContext<C> {
        ReplicationContext {
            id: self.id.clone(),
            target,
            session_id,
            config: self.config.clone(),
            tx_notify: self.tx_notification.clone(),
        }
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

        match cmd {
            Command::UpdateIOProgress { io_id, .. } => {
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
                self.runtime_stats.append_batch.record(entry_count);

                let io_id = IOId::new_log_io(vote, Some(last_log_id));
                let callback = IOFlushed::new(io_id.clone(), self.tx_io_completed.clone());

                // Mark this IO request as submitted,
                // other commands relying on it can then be processed.
                // For example,
                // `Replicate` command cannot run until this IO request is submitted(no need to be flushed),
                // because it needs to read the log entry from the log store.
                //
                // The `submit` state must be updated before calling `append()`,
                // because `append()` may call the callback before returning.
                self.engine.state.log_progress_mut().submit(io_id);

                // Submit IO request, do not wait for the response.
                self.log_store.append(entries, callback).await.sto_write_logs()?;
            }
            Command::SaveVote { vote } => {
                self.engine.state.log_progress_mut().submit(IOId::new(&vote));
                self.log_store.save_vote(&vote).await.sto_write_vote()?;

                let _ = self
                    .tx_notification
                    .send(Notification::LocalIO {
                        io_id: IOId::new(&vote),
                    })
                    .await;

                // If a non-committed vote is saved,
                // there may be a candidate waiting for the response.
                if let VoteStatus::Pending(non_committed) = vote.clone().into_vote_status() {
                    let _ = self
                        .tx_notification
                        .send(Notification::VoteResponse {
                            target: self.id.clone(),
                            // last_log_id is not used when sending VoteRequest to local node
                            resp: VoteResponse::new(vote, None, true),
                            candidate_vote: non_committed,
                        })
                        .await;
                }
            }
            Command::PurgeLog { upto } => {
                self.log_store.purge(upto.clone()).await.sto_write_logs()?;
                self.engine.state.io_state_mut().update_purged(Some(upto));
            }
            Command::TruncateLog { since } => {
                self.log_store.truncate(since.clone()).await.sto_write_logs()?;

                // Inform clients waiting for logs to be applied.
                let leader_id = self.current_leader();
                let leader_node = self.get_leader_node(leader_id.clone());

                for (log_index, tx) in self.client_responders.drain_from(since.index()) {
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
                for node in self.replications.values() {
                    let _ = node.tx_repl.send(Replicate::Committed {
                        committed: committed.clone(),
                    });
                }
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
                let _ = node.tx_repl.send(req);
            }
            Command::ReplicateSnapshot { target, inflight_id } => {
                let node = self.replications.get(&target).expect("replication to target node exists");

                let snapshot_reader = self.sm_handle.new_snapshot_reader();
                let session_id = node.session_id.clone();
                let replication_task_state = self.new_replication_task_state(session_id, target.clone());

                let target_node = self.engine.state.membership_state.effective().get_node(&target).unwrap();
                let snapshot_network = self.network_factory.new_client(target.clone(), target_node).await;

                let handle = SnapshotTransmitter::<C, N>::spawn(
                    replication_task_state,
                    snapshot_network,
                    snapshot_reader,
                    inflight_id,
                );

                let node = self.replications.get_mut(&target).expect("replication to target node exists");
                // TODO: it is not cleaned when snapshot transmission is done.
                node.snapshot_transmit_handle = Some(handle);
            }
            Command::BroadcastTransferLeader { req } => self.broadcast_transfer_leader(req).await,

            Command::RebuildReplicationStreams { targets } => {
                self.remove_all_replication().await;

                for ReplicationProgress(target, matching) in targets.iter() {
                    let handle = self.spawn_replication_stream(target.clone(), matching.clone()).await;
                    self.replications.insert(target.clone(), handle);
                }

                let effective = self.engine.state.membership_state.effective().clone();

                let nodes = targets.into_iter().map(|p| {
                    let node_id = p.0;
                    (node_id.clone(), effective.get_node(&node_id).unwrap().clone())
                });

                self.heartbeat_handle.spawn_workers(&mut self.network_factory, &self.tx_notification, nodes).await;
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
                self.sm_handle.send(command).map_err(|_e| {
                    StorageError::write_state_machine(AnyError::error("cannot send to sm::Worker".to_string()))
                })?;
            }
            Command::Respond { resp: send, .. } => {
                send.send();
            }
        }

        Ok(None)
    }
}
