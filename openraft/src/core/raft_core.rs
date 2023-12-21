use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::marker::PhantomData;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyerror::AnyError;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use futures::TryFutureExt;
use macros::add_async_trait;
use maplit::btreeset;
use tokio::select;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::config::Config;
use crate::config::RuntimeConfig;
use crate::core::balancer::Balancer;
use crate::core::command_state::CommandState;
use crate::core::notify::Notify;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::core::raft_msg::AppendEntriesTx;
use crate::core::raft_msg::ClientReadTx;
use crate::core::raft_msg::ClientWriteTx;
use crate::core::raft_msg::InstallSnapshotTx;
use crate::core::raft_msg::RaftMsg;
use crate::core::raft_msg::ResultSender;
use crate::core::raft_msg::VoteTx;
use crate::core::sm;
use crate::core::sm::CommandSeq;
use crate::core::ServerState;
use crate::display_ext::DisplayOption;
use crate::display_ext::DisplaySlice;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::Engine;
use crate::engine::Respond;
use crate::entry::FromAppData;
use crate::entry::RaftEntry;
use crate::error::ClientWriteError;
use crate::error::Fatal;
use crate::error::ForwardToLeader;
use crate::error::InitializeError;
use crate::error::QuorumNotEnough;
use crate::error::RPCError;
use crate::error::Timeout;
use crate::log_id::LogIdOptionExt;
use crate::log_id::RaftLogId;
use crate::metrics::RaftMetrics;
use crate::metrics::ReplicationMetrics;
use crate::network::RPCOption;
use crate::network::RPCTypes;
use crate::network::RaftNetwork;
use crate::network::RaftNetworkFactory;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::quorum::QuorumSet;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::ClientWriteResponse;
use crate::raft::InstallSnapshotRequest;
use crate::raft::VoteRequest;
use crate::raft_state::LogStateReader;
use crate::replication;
use crate::replication::request::Replicate;
use crate::replication::response::ReplicationResult;
use crate::replication::ReplicationCore;
use crate::replication::ReplicationHandle;
use crate::replication::ReplicationSessionId;
use crate::runtime::RaftRuntime;
use crate::storage::LogFlushed;
use crate::storage::RaftLogReaderExt;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::InstantOf;
use crate::utime::UTime;
use crate::AsyncRuntime;
use crate::ChangeMembers;
use crate::Instant;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StorageIOError;
use crate::Vote;

/// A temp struct to hold the data for a node that is being applied.
#[derive(Debug)]
pub(crate) struct ApplyingEntry<NID: NodeId, N: Node> {
    log_id: LogId<NID>,
    membership: Option<Membership<NID, N>>,
}

impl<NID: NodeId, N: Node> ApplyingEntry<NID, N> {
    pub(crate) fn new(log_id: LogId<NID>, membership: Option<Membership<NID, N>>) -> Self {
        Self { log_id, membership }
    }
}

/// The result of applying log entries to state machine.
pub(crate) struct ApplyResult<C: RaftTypeConfig> {
    pub(crate) since: u64,
    pub(crate) end: u64,
    pub(crate) last_applied: LogId<C::NodeId>,
    pub(crate) applying_entries: Vec<ApplyingEntry<C::NodeId, C::Node>>,
    pub(crate) apply_results: Vec<C::R>,
}

impl<C: RaftTypeConfig> Debug for ApplyResult<C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplyResult")
            .field("since", &self.since)
            .field("end", &self.end)
            .field("last_applied", &self.last_applied)
            .finish()
    }
}

/// Data for a Leader.
///
/// It is created when RaftCore enters leader state, and will be dropped when it quits leader state.
pub(crate) struct LeaderData<C: RaftTypeConfig> {
    /// A mapping of node IDs the replication state of the target node.
    // TODO(xp): make it a field of RaftCore. it does not have to belong to leader.
    //           It requires the Engine to emit correct add/remove replication commands
    pub(super) replications: BTreeMap<C::NodeId, ReplicationHandle<C>>,

    /// The time to send next heartbeat.
    pub(crate) next_heartbeat: <C::AsyncRuntime as AsyncRuntime>::Instant,
}

impl<C: RaftTypeConfig> LeaderData<C> {
    pub(crate) fn new() -> Self {
        Self {
            replications: BTreeMap::new(),
            next_heartbeat: <C::AsyncRuntime as AsyncRuntime>::Instant::now(),
        }
    }
}

// TODO: remove SM
/// The core type implementing the Raft protocol.
pub struct RaftCore<C, N, LS, SM>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    /// This node's ID.
    pub(crate) id: C::NodeId,

    /// This node's runtime config.
    pub(crate) config: Arc<Config>,

    pub(crate) runtime_config: Arc<RuntimeConfig>,

    /// The `RaftNetworkFactory` implementation.
    pub(crate) network: N,

    /// The [`RaftLogStorage`] implementation.
    pub(crate) log_store: LS,

    /// A controlling handle to the [`RaftStateMachine`] worker.
    pub(crate) sm_handle: sm::Handle<C>,

    pub(crate) engine: Engine<C>,

    /// Channels to send result back to client when logs are applied.
    pub(crate) client_resp_channels: BTreeMap<u64, ClientWriteTx<C>>,

    pub(crate) leader_data: Option<LeaderData<C>>,

    #[allow(dead_code)]
    pub(crate) tx_api: mpsc::UnboundedSender<RaftMsg<C>>,
    pub(crate) rx_api: mpsc::UnboundedReceiver<RaftMsg<C>>,

    /// A Sender to send callback by other components to [`RaftCore`], when an action is finished,
    /// such as flushing log to disk, or applying log entries to state machine.
    pub(crate) tx_notify: mpsc::UnboundedSender<Notify<C>>,

    /// A Receiver to receive callback from other components.
    pub(crate) rx_notify: mpsc::UnboundedReceiver<Notify<C>>,

    pub(crate) tx_metrics: watch::Sender<RaftMetrics<C::NodeId, C::Node>>,

    pub(crate) command_state: CommandState,

    pub(crate) span: Span,

    pub(crate) _p: PhantomData<SM>,
}

impl<C, N, LS, SM> RaftCore<C, N, LS, SM>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    /// The main loop of the Raft protocol.
    pub(crate) async fn main(mut self, rx_shutdown: oneshot::Receiver<()>) -> Result<(), Fatal<C::NodeId>> {
        let span = tracing::span!(parent: &self.span, Level::DEBUG, "main");
        let res = self.do_main(rx_shutdown).instrument(span).await;

        // Flush buffered metrics
        self.report_metrics(None);

        tracing::info!("update the metrics for shutdown");
        {
            let mut curr = self.tx_metrics.borrow().clone();
            curr.state = ServerState::Shutdown;

            if let Err(err) = &res {
                tracing::error!(?err, "quit RaftCore::main on error");
                curr.running_state = Err(err.clone());
            }

            let _ = self.tx_metrics.send(curr);
        }

        res
    }

    #[tracing::instrument(level="trace", skip_all, fields(id=display(self.id), cluster=%self.config.cluster_name))]
    async fn do_main(&mut self, rx_shutdown: oneshot::Receiver<()>) -> Result<(), Fatal<C::NodeId>> {
        tracing::debug!("raft node is initializing");

        self.engine.startup();
        // It may not finish running all of the commands, if there is a command waiting for a callback.
        self.run_engine_commands().await?;

        // Initialize metrics.
        self.report_metrics(None);

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
    pub(super) async fn handle_check_is_leader_request(&mut self, tx: ClientReadTx<C>) {
        // Setup sentinel values to track when we've received majority confirmation of leadership.

        let resp = {
            let read_log_id = self.engine.state.get_read_log_id().copied();

            // TODO: this applied is a little stale when being returned to client.
            //       Fix this when the following heartbeats are replaced with calling RaftNetwork.
            let applied = self.engine.state.io_applied().copied();

            (read_log_id, applied)
        };

        let my_id = self.id;
        let my_vote = *self.engine.state.vote_ref();
        let ttl = Duration::from_millis(self.config.heartbeat_interval);
        let eff_mem = self.engine.state.membership_state.effective().clone();
        let core_tx = self.tx_notify.clone();

        let mut granted = btreeset! {my_id};

        if eff_mem.is_quorum(granted.iter()) {
            let _ = tx.send(Ok(resp));
            return;
        }

        // Spawn parallel requests, all with the standard timeout for heartbeats.
        let mut pending = FuturesUnordered::new();

        let voter_progresses = {
            let l = &self.engine.internal_server_state.leading().unwrap();
            l.progress.iter().filter(|(id, _v)| l.progress.is_voter(id) == Some(true))
        };

        for (target, progress) in voter_progresses {
            let target = *target;

            if target == my_id {
                continue;
            }

            let rpc = AppendEntriesRequest {
                vote: my_vote,
                prev_log_id: progress.matching,
                entries: vec![],
                leader_commit: self.engine.state.committed().copied(),
            };

            // Safe unwrap(): target is in membership
            let target_node = eff_mem.get_node(&target).unwrap().clone();
            let mut client = self.network.new_client(target, &target_node).await;

            let option = RPCOption::new(ttl);

            let fu = async move {
                let outer_res = C::AsyncRuntime::timeout(ttl, client.append_entries(rpc, option)).await;
                match outer_res {
                    Ok(append_res) => match append_res {
                        Ok(x) => Ok((target, x)),
                        Err(err) => Err((target, err)),
                    },
                    Err(_timeout) => {
                        let timeout_err = Timeout {
                            action: RPCTypes::AppendEntries,
                            id: my_id,
                            target,
                            timeout: ttl,
                        };

                        Err((target, RPCError::Timeout(timeout_err)))
                    }
                }
            };

            let fu = fu.instrument(tracing::debug_span!("spawn_is_leader", target = target.to_string()));
            let task = C::AsyncRuntime::spawn(fu).map_err(move |err| (target, err));

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
                        vote > my_vote,
                        "committed vote({}) has total order relation with other votes({})",
                        my_vote,
                        vote
                    );

                    let send_res = core_tx.send(Notify::HigherVote {
                        target,
                        higher: vote,
                        sender_vote: my_vote,
                    });

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
                cluster: eff_mem.membership().summary(),
                got: granted,
            }
            .into()));
        };

        // TODO: do not spawn, manage read requests with a queue by RaftCore

        // False positive lint warning(`non-binding `let` on a future`): https://github.com/rust-lang/rust-clippy/issues/9932
        #[allow(clippy::let_underscore_future)]
        let _ = C::AsyncRuntime::spawn(waiting_fu.instrument(tracing::debug_span!("spawn_is_leader_waiting")));
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
        changes: ChangeMembers<C::NodeId, C::Node>,
        retain: bool,
        tx: ResultSender<ClientWriteResponse<C>, ClientWriteError<C::NodeId, C::Node>>,
    ) {
        let res = self.engine.state.membership_state.change_handler().apply(changes, retain);
        let new_membership = match res {
            Ok(x) => x,
            Err(e) => {
                let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(e)));
                return;
            }
        };

        let ent = C::Entry::new_membership(LogId::default(), new_membership);
        self.write_entry(ent, Some(tx));
    }

    /// Write a log entry to the cluster through raft protocol.
    ///
    /// I.e.: append the log entry to local store, forward it to a quorum(including the leader),
    /// waiting for it to be committed and applied.
    ///
    /// The result of applying it to state machine is sent to `resp_tx`, if it is not `None`.
    /// The calling side may not receive a result from `resp_tx`, if raft is shut down.
    #[tracing::instrument(level = "debug", skip_all, fields(id = display(self.id)))]
    pub fn write_entry(&mut self, entry: C::Entry, resp_tx: Option<ClientWriteTx<C>>) -> bool {
        tracing::debug!(payload = display(&entry), "write_entry");

        let (mut lh, tx) = if let Some((lh, tx)) = self.engine.get_leader_handler_or_reject(resp_tx) {
            (lh, tx)
        } else {
            return false;
        };

        let entries = vec![entry];
        // TODO: it should returns membership config error etc. currently this is done by the
        //       caller.
        lh.leader_append_entries(entries);
        let index = lh.state.last_log_id().unwrap().index;

        // Install callback channels.
        if let Some(tx) = tx {
            self.client_resp_channels.insert(index, tx);
        }

        true
    }

    /// Send a heartbeat message to every followers/learners.
    ///
    /// Currently heartbeat is a blank log
    #[tracing::instrument(level = "debug", skip_all, fields(id = display(self.id)))]
    pub fn send_heartbeat(&mut self, emitter: impl Display) -> bool {
        tracing::debug!(
            now = debug(<C::AsyncRuntime as AsyncRuntime>::Instant::now()),
            "send_heartbeat"
        );

        let mut lh = if let Some((lh, _)) =
            self.engine.get_leader_handler_or_reject::<(), ClientWriteError<C::NodeId, C::Node>>(None)
        {
            lh
        } else {
            tracing::debug!(
                now = debug(<C::AsyncRuntime as AsyncRuntime>::Instant::now()),
                "{} failed to send heartbeat",
                emitter
            );
            return false;
        };

        lh.send_heartbeat();

        tracing::debug!("{} triggered sending heartbeat", emitter);
        true
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub fn flush_metrics(&mut self) {
        let leader_metrics = if let Some(leader) = self.engine.internal_server_state.leading() {
            let prog = &leader.progress;
            Some(prog.iter().map(|(id, p)| (*id, *p.borrow())).collect())
        } else {
            None
        };
        self.report_metrics(leader_metrics);
    }

    /// Report a metrics payload on the current state of the Raft node.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn report_metrics(&self, replication: Option<ReplicationMetrics<C::NodeId>>) {
        let st = &self.engine.state;

        let m = RaftMetrics {
            running_state: Ok(()),
            id: self.id,

            // --- data ---
            current_term: st.vote_ref().leader_id().get_term(),
            vote: *st.io_state().vote(),
            last_log_index: st.last_log_id().index(),
            last_applied: st.io_applied().copied(),
            snapshot: st.io_snapshot_last_log_id().copied(),
            purged: st.io_purged().copied(),

            // --- cluster ---
            state: st.server_state,
            current_leader: self.current_leader(),
            membership_config: st.membership_state.effective().stored_membership().clone(),

            // --- replication ---
            replication,
        };

        tracing::debug!("report_metrics: {}", m.summary());
        let res = self.tx_metrics.send(m);

        if let Err(err) = res {
            tracing::error!(error=%err, id=display(self.id), "error reporting metrics");
        }
    }

    /// Handle the admin command `initialize`.
    ///
    /// It is allowed to initialize only when `last_log_id.is_none()` and `vote==(0,0)`.
    /// See: [Conditions for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(crate) fn handle_initialize(
        &mut self,
        member_nodes: BTreeMap<C::NodeId, C::Node>,
        tx: ResultSender<(), InitializeError<C::NodeId, C::Node>>,
    ) {
        tracing::debug!(member_nodes = debug(&member_nodes), "{}", func_name!());

        let membership = Membership::from(member_nodes);

        let entry = C::Entry::new_membership(LogId::default(), membership);
        let res = self.engine.initialize(entry);
        self.engine.output.push_command(Command::Respond {
            when: None,
            resp: Respond::new(res, tx),
        });
    }

    /// Invoked by leader to send chunks of a snapshot to a follower.
    ///
    /// Leaders always send chunks in order. It is important to note that, according to the Raft
    /// spec, a node may only have one snapshot at any time. As snapshot contents are application
    /// specific.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_install_snapshot_request(
        &mut self,
        req: InstallSnapshotRequest<C>,
        tx: InstallSnapshotTx<C::NodeId>,
    ) {
        tracing::info!(req = display(req.summary()), "{}", func_name!());
        self.engine.handle_install_snapshot(req, tx);
    }

    /// Trigger a snapshot building(log compaction) job if there is no pending building job.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn trigger_snapshot(&mut self) {
        tracing::debug!("{}", func_name!());
        self.engine.snapshot_handler().trigger_snapshot();
    }

    /// Reject a request due to the Raft node being in a state which prohibits the request.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    pub(crate) fn reject_with_forward_to_leader<T, E>(&self, tx: ResultSender<T, E>)
    where E: From<ForwardToLeader<C::NodeId, C::Node>> {
        let mut leader_id = self.current_leader();
        let leader_node = self.get_leader_node(leader_id);

        // Leader is no longer a node in the membership config.
        if leader_node.is_none() {
            leader_id = None;
        }

        let err = ForwardToLeader { leader_id, leader_node };

        let _ = tx.send(Err(err.into()));
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn current_leader(&self) -> Option<C::NodeId> {
        tracing::debug!(
            self_id = display(self.id),
            vote = display(self.engine.state.vote_ref().summary()),
            "get current_leader"
        );

        let vote = self.engine.state.vote_ref();

        if !vote.is_committed() {
            return None;
        }

        // Safe unwrap(): vote that is committed has to already have voted for some node.
        let id = vote.leader_id().voted_for().unwrap();

        // TODO: `is_voter()` is slow, maybe cache `current_leader`,
        //       e.g., only update it when membership or vote changes
        if self.engine.state.membership_state.effective().is_voter(&id) {
            Some(id)
        } else {
            tracing::debug!("id={} is not a voter", id);
            None
        }
    }

    pub(crate) fn get_leader_node(&self, leader_id: Option<C::NodeId>) -> Option<C::Node> {
        let leader_id = match leader_id {
            None => return None,
            Some(x) => x,
        };

        self.engine.state.membership_state.effective().get_node(&leader_id).cloned()
    }

    /// A temp wrapper to make non-blocking `append_to_log` a blocking.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn append_to_log<I>(
        &mut self,
        entries: I,
        last_log_id: LogId<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>>
    where
        I: IntoIterator<Item = C::Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        tracing::debug!("append_to_log");

        let (tx, rx) = oneshot::channel();
        let callback = LogFlushed::new(Some(last_log_id), tx);
        self.log_store.append(entries, callback).await?;
        rx.await
            .map_err(|e| StorageIOError::write_logs(AnyError::error(e)))?
            .map_err(|e| StorageIOError::write_logs(AnyError::error(e)))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn apply_to_state_machine(
        &mut self,
        seq: CommandSeq,
        since: u64,
        upto_index: u64,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(upto_index = display(upto_index), "apply_to_state_machine");

        let end = upto_index + 1;

        debug_assert!(
            since <= end,
            "last_applied index {} should <= committed index {}",
            since,
            end
        );

        if since == end {
            return Ok(());
        }

        let entries = self.log_store.get_log_entries(since..end).await?;
        tracing::debug!(
            entries = display(DisplaySlice::<_>(entries.as_slice())),
            "about to apply"
        );

        let last_applied = *entries[entries.len() - 1].get_log_id();

        let cmd = sm::Command::apply(entries).with_seq(seq);
        self.sm_handle.send(cmd).map_err(|e| StorageIOError::apply(last_applied, AnyError::error(e)))?;

        Ok(())
    }

    /// When received results of applying log entries to the state machine, send back responses to
    /// the callers that proposed the entries.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_apply_result(&mut self, res: ApplyResult<C>) {
        tracing::debug!(last_applied = display(res.last_applied), "{}", func_name!());

        let mut results = res.apply_results.into_iter();
        let mut applying_entries = res.applying_entries.into_iter();

        for log_index in res.since..res.end {
            let ent = applying_entries.next().unwrap();
            let apply_res = results.next().unwrap();
            let tx = self.client_resp_channels.remove(&log_index);

            Self::send_response(ent, apply_res, tx);
        }
    }

    /// Send result of applying a log entry to its client.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn send_response(entry: ApplyingEntry<C::NodeId, C::Node>, resp: C::R, tx: Option<ClientWriteTx<C>>) {
        tracing::debug!(entry = debug(&entry), "send_response");

        let tx = match tx {
            None => return,
            Some(x) => x,
        };

        let membership = entry.membership;

        let res = Ok(ClientWriteResponse {
            log_id: entry.log_id,
            data: resp,
            membership,
        });

        let send_res = tx.send(res);
        tracing::debug!(
            "send client response through tx, send_res is error: {}",
            send_res.is_err()
        );
    }

    /// Spawn a new replication stream returning its replication state handle.
    #[tracing::instrument(level = "debug", skip(self))]
    #[allow(clippy::type_complexity)]
    pub(crate) async fn spawn_replication_stream(
        &mut self,
        target: C::NodeId,
        progress_entry: ProgressEntry<C::NodeId>,
    ) -> ReplicationHandle<C> {
        // Safe unwrap(): target must be in membership
        let target_node = self.engine.state.membership_state.effective().get_node(&target).unwrap();

        let membership_log_id = self.engine.state.membership_state.effective().log_id();
        let network = self.network.new_client(target, target_node).await;

        let session_id = ReplicationSessionId::new(*self.engine.state.vote_ref(), *membership_log_id);

        ReplicationCore::<C, N, LS>::spawn(
            target,
            session_id,
            self.config.clone(),
            self.engine.state.committed().copied(),
            progress_entry.matching,
            network,
            self.log_store.get_log_reader().await,
            self.tx_notify.clone(),
            tracing::span!(parent: &self.span, Level::DEBUG, "replication", id=display(self.id), target=display(target)),
        )
    }

    /// Remove all replication.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn remove_all_replication(&mut self) {
        tracing::info!("remove all replication");

        if let Some(l) = &mut self.leader_data {
            let nodes = std::mem::take(&mut l.replications);

            tracing::debug!(
                targets = debug(nodes.iter().map(|x| *x.0).collect::<Vec<_>>()),
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
        } else {
            unreachable!("it has to be a leader!!!");
        };
    }

    /// Run as many commands as possible.
    ///
    /// If there is a command that waits for a callback, just return and wait for
    /// next RaftMsg.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn run_engine_commands(&mut self) -> Result<(), StorageError<C::NodeId>> {
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!("queued commands: start...");
            for c in self.engine.output.iter_commands() {
                tracing::debug!("queued commands: {:?}", c);
            }
            tracing::debug!("queued commands: end...");
        }

        while let Some(cmd) = self.engine.output.pop_command() {
            tracing::debug!("run command: {:?}", cmd);

            let res = self.run_command(cmd).await?;

            if let Some(cmd) = res {
                tracing::debug!("early return: postpone command: {:?}", cmd);
                self.engine.output.postpone_command(cmd);

                if tracing::enabled!(Level::DEBUG) {
                    for c in self.engine.output.iter_commands().take(8) {
                        tracing::debug!("postponed, first 8 queued commands: {:?}", c);
                    }
                }

                return Ok(());
            }
        }

        Ok(())
    }

    /// Run an event handling loop
    #[tracing::instrument(level="debug", skip_all, fields(id=display(self.id)))]
    async fn runtime_loop(&mut self, mut rx_shutdown: oneshot::Receiver<()>) -> Result<(), Fatal<C::NodeId>> {
        // Ratio control the ratio of number of RaftMsg to process to number of Notify to process.
        let mut balancer = Balancer::new(10_000);

        loop {
            self.flush_metrics();

            // In each loop, it does not have to check rx_shutdown and flush metrics for every RaftMsg
            // processed.
            // In each loop, the first step is blocking waiting for any message from any channel.
            // Then if there is any message, process as many as possible to maximize throughput.

            select! {
                // Check shutdown in each loop first so that a message flood in `tx_api` won't block shutting down.
                // `select!` without `biased` provides a random fairness.
                // We want to check shutdown prior to other channels.
                // See: https://docs.rs/tokio/latest/tokio/macro.select.html#fairness
                biased;

                _ = &mut rx_shutdown => {
                    tracing::info!("recv from rx_shutdown");
                    return Err(Fatal::Stopped);
                }

                notify_res = self.rx_notify.recv() => {
                    match notify_res {
                        Some(notify) => self.handle_notify(notify)?,
                        None => {
                            tracing::error!("all rx_notify senders are dropped");
                            return Err(Fatal::Stopped);
                        }
                    };
                }

                msg_res = self.rx_api.recv() => {
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
            let notify_processed = self.process_notify(balancer.notify()).await?;

            // If one of the channel consumed all its budget, re-balance the budget ratio.

            #[allow(clippy::collapsible_else_if)]
            if notify_processed == balancer.notify() {
                tracing::info!("there may be more Notify to process, increase Notify ratio");
                balancer.increase_notify();
            } else {
                if raft_msg_processed == balancer.raft_msg() {
                    tracing::info!("there may be more RaftMsg to process, increase RaftMsg ratio");
                    balancer.increase_raft_msg();
                }
            }
        }
    }

    /// Process RaftMsg as many as possible.
    ///
    /// It returns the number of processed message.
    /// If the input channel is closed, it returns `Fatal::Stopped`.
    async fn process_raft_msg(&mut self, at_most: u64) -> Result<u64, Fatal<C::NodeId>> {
        for i in 0..at_most {
            let res = self.rx_api.try_recv();
            let msg = match res {
                Ok(msg) => msg,
                Err(e) => match e {
                    mpsc::error::TryRecvError::Empty => {
                        tracing::debug!("all RaftMsg are processed, wait for more");
                        return Ok(i + 1);
                    }
                    mpsc::error::TryRecvError::Disconnected => {
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

    /// Process Notify as many as possible.
    ///
    /// It returns the number of processed notifications.
    /// If the input channel is closed, it returns `Fatal::Stopped`.
    async fn process_notify(&mut self, at_most: u64) -> Result<u64, Fatal<C::NodeId>> {
        for i in 0..at_most {
            let res = self.rx_notify.try_recv();
            let notify = match res {
                Ok(msg) => msg,
                Err(e) => match e {
                    mpsc::error::TryRecvError::Empty => {
                        tracing::debug!("all Notify are processed, wait for more");
                        return Ok(i + 1);
                    }
                    mpsc::error::TryRecvError::Disconnected => {
                        tracing::error!("rx_notify is disconnected, quit");
                        return Err(Fatal::Stopped);
                    }
                },
            };

            self.handle_notify(notify)?;

            // TODO: does run_engine_commands() run too frequently?
            //       to run many commands in one shot, it is possible to batch more commands to gain
            //       better performance.

            self.run_engine_commands().await?;
        }

        tracing::debug!("at_most({}) reached, there are more queued Notify to process", at_most);

        Ok(at_most)
    }

    /// Spawn parallel vote requests to all cluster members.
    #[tracing::instrument(level = "trace", skip_all, fields(vote=vote_req.summary()))]
    async fn spawn_parallel_vote_requests(&mut self, vote_req: &VoteRequest<C::NodeId>) {
        let members = self.engine.state.membership_state.effective().voter_ids();

        let vote = vote_req.vote;

        for target in members {
            if target == self.id {
                continue;
            }

            let req = vote_req.clone();

            // Safe unwrap(): target must be in membership
            let target_node = self.engine.state.membership_state.effective().get_node(&target).unwrap().clone();
            let mut client = self.network.new_client(target, &target_node).await;

            let tx = self.tx_notify.clone();

            let ttl = Duration::from_millis(self.config.election_timeout_min);
            let id = self.id;
            let option = RPCOption::new(ttl);

            // False positive lint warning(`non-binding `let` on a future`): https://github.com/rust-lang/rust-clippy/issues/9932
            #[allow(clippy::let_underscore_future)]
            let _ = C::AsyncRuntime::spawn(
                async move {
                    let tm_res = C::AsyncRuntime::timeout(ttl, client.vote(req, option)).await;
                    let res = match tm_res {
                        Ok(res) => res,

                        Err(_timeout) => {
                            let timeout_err = Timeout {
                                action: RPCTypes::Vote,
                                id,
                                target,
                                timeout: ttl,
                            };
                            tracing::error!({error = %timeout_err, target = display(target)}, "timeout");
                            return;
                        }
                    };

                    match res {
                        Ok(resp) => {
                            let _ = tx.send(Notify::VoteResponse {
                                target,
                                resp,
                                sender_vote: vote,
                            });
                        }
                        Err(err) => tracing::error!({error=%err, target=display(target)}, "while requesting vote"),
                    }
                }
                .instrument(tracing::debug_span!(
                    parent: &Span::current(),
                    "send_vote_req",
                    target = display(target)
                )),
            );
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn handle_vote_request(&mut self, req: VoteRequest<C::NodeId>, tx: VoteTx<C::NodeId>) {
        tracing::info!(req = display(req.summary()), func = func_name!());

        let resp = self.engine.handle_vote_req(req);
        self.engine.output.push_command(Command::Respond {
            when: None,
            resp: Respond::new(Ok(resp), tx),
        });
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn handle_append_entries_request(
        &mut self,
        req: AppendEntriesRequest<C>,
        tx: AppendEntriesTx<C::NodeId>,
    ) {
        tracing::debug!(req = display(req.summary()), func = func_name!());

        let is_ok = self.engine.handle_append_entries(&req.vote, req.prev_log_id, req.entries, Some(tx));

        if is_ok {
            self.engine.handle_commit_entries(req.leader_commit);
        }
    }

    // TODO: Make this method non-async. It does not need to run any async command in it.
    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = debug(self.engine.state.server_state), id=display(self.id)))]
    pub(crate) async fn handle_api_msg(&mut self, msg: RaftMsg<C>) {
        tracing::debug!("recv from rx_api: {}", msg.summary());

        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                self.handle_append_entries_request(rpc, tx);
            }
            RaftMsg::RequestVote { rpc, tx } => {
                let now = <C::AsyncRuntime as AsyncRuntime>::Instant::now();
                tracing::info!(
                    now = debug(now),
                    vote_request = display(rpc.summary()),
                    "received RaftMsg::RequestVote: {}",
                    func_name!()
                );

                self.handle_vote_request(rpc, tx);
            }
            RaftMsg::InstallSnapshot { rpc, tx } => {
                tracing::info!(
                    req = display(rpc.summary()),
                    "received RaftMst::InstallSnapshot: {}",
                    func_name!()
                );

                self.handle_install_snapshot_request(rpc, tx);
            }
            RaftMsg::CheckIsLeaderRequest { tx } => {
                if self.engine.state.is_leader(&self.engine.config.id) {
                    self.handle_check_is_leader_request(tx).await;
                } else {
                    self.reject_with_forward_to_leader(tx);
                }
            }
            RaftMsg::ClientWriteRequest { app_data, tx } => {
                self.write_entry(C::Entry::from_app_data(app_data), Some(tx));
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
            RaftMsg::ExternalRequest { req } => {
                req(&self.engine.state);
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
                    ExternalCommand::PurgeLog { upto } => {
                        self.engine.trigger_purge_log(upto);
                    }
                }
            }
        };
    }

    // TODO: Make this method non-async. It does not need to run any async command in it.
    #[tracing::instrument(level = "debug", skip_all, fields(state = debug(self.engine.state.server_state), id=display(self.id)))]
    pub(crate) fn handle_notify(&mut self, notify: Notify<C>) -> Result<(), Fatal<C::NodeId>> {
        tracing::debug!("recv from rx_notify: {}", notify.summary());

        match notify {
            Notify::VoteResponse {
                target,
                resp,
                sender_vote: vote,
            } => {
                let now = <C::AsyncRuntime as AsyncRuntime>::Instant::now();

                tracing::info!(
                    now = debug(now),
                    resp = display(resp.summary()),
                    "received Notify::VoteResponse: {}",
                    func_name!()
                );

                if self.does_vote_match(&vote, "VoteResponse") {
                    self.engine.handle_vote_resp(target, resp);
                }
            }

            Notify::HigherVote {
                target,
                higher,
                sender_vote: vote,
            } => {
                tracing::info!(
                    target = display(target),
                    higher_vote = display(&higher),
                    sending_vote = display(&vote),
                    "received Notify::HigherVote: {}",
                    func_name!()
                );

                if self.does_vote_match(&vote, "HigherVote") {
                    // Rejected vote change is ok.
                    let _ = self.engine.vote_handler().update_vote(&higher);
                }
            }

            Notify::Tick { i } => {
                // check every timer

                let now = <C::AsyncRuntime as AsyncRuntime>::Instant::now();
                tracing::debug!("received tick: {}, now: {:?}", i, now);

                self.handle_tick_election();

                // TODO: test: fixture: make isolated_nodes a single-way isolating.

                // Leader send heartbeat
                let heartbeat_at = self.leader_data.as_ref().map(|x| x.next_heartbeat);
                if let Some(t) = heartbeat_at {
                    if now >= t {
                        if self.runtime_config.enable_heartbeat.load(Ordering::Relaxed) {
                            self.send_heartbeat("tick");
                        }

                        // Install next heartbeat
                        if let Some(l) = &mut self.leader_data {
                            l.next_heartbeat = <C::AsyncRuntime as AsyncRuntime>::Instant::now()
                                + Duration::from_millis(self.config.heartbeat_interval);
                        }
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

            Notify::Network { response } => {
                //
                match response {
                    replication::Response::Progress {
                        target,
                        request_id: id,
                        result,
                        session_id,
                    } => {
                        // If vote or membership changes, ignore the message.
                        // There is chance delayed message reports a wrong state.
                        if self.does_replication_session_match(&session_id, "UpdateReplicationMatched") {
                            self.handle_replication_progress(target, id, result);
                        }
                    }

                    replication::Response::StorageError { error } => {
                        tracing::error!(
                            error = display(&error),
                            "received Notify::ReplicationStorageError: {}",
                            func_name!()
                        );

                        return Err(Fatal::from(error));
                    }

                    replication::Response::HigherVote { target, higher, vote } => {
                        tracing::info!(
                            target = display(target),
                            higher_vote = display(&higher),
                            sending_vote = display(&vote),
                            "received Notify::HigherVote: {}",
                            func_name!()
                        );

                        if self.does_vote_match(&vote, "HigherVote") {
                            // Rejected vote change is ok.
                            let _ = self.engine.vote_handler().update_vote(&higher);
                        }
                    }
                }
            }

            Notify::StateMachine { command_result } => {
                tracing::debug!("sm::StateMachine command result: {:?}", command_result);

                let seq = command_result.command_seq;
                let res = command_result.result?;

                match res {
                    // BuildSnapshot is a read operation that does not have to be serialized by
                    // sm::Worker. Thus it may finish out of order.
                    sm::Response::BuildSnapshot(_) => {}
                    _ => {
                        debug_assert!(
                            self.command_state.finished_sm_seq < seq,
                            "sm::StateMachine command result is out of order: expect {} < {}",
                            self.command_state.finished_sm_seq,
                            seq
                        );
                    }
                }
                self.command_state.finished_sm_seq = seq;

                match res {
                    sm::Response::BuildSnapshot(meta) => {
                        tracing::info!(
                            "sm::StateMachine command done: BuildSnapshot: {}: {}",
                            meta.summary(),
                            func_name!()
                        );

                        // Update in-memory state first, then the io state.
                        // In-memory state should always be ahead or equal to the io state.

                        let last_log_id = meta.last_log_id;
                        self.engine.finish_building_snapshot(meta);

                        let st = self.engine.state.io_state_mut();
                        st.update_snapshot(last_log_id);
                    }
                    sm::Response::ReceiveSnapshotChunk(_) => {
                        tracing::info!("sm::StateMachine command done: ReceiveSnapshotChunk: {}", func_name!());
                    }
                    sm::Response::InstallSnapshot(meta) => {
                        tracing::info!(
                            "sm::StateMachine command done: InstallSnapshot: {}: {}",
                            meta.summary(),
                            func_name!()
                        );

                        if let Some(meta) = meta {
                            let st = self.engine.state.io_state_mut();
                            st.update_applied(meta.last_log_id);
                            st.update_snapshot(meta.last_log_id);
                        }
                    }
                    sm::Response::Apply(res) => {
                        self.engine.state.io_state_mut().update_applied(Some(res.last_applied));

                        self.handle_apply_result(res);
                    }
                }
            }
        };
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    fn handle_tick_election(&mut self) {
        let now = <C::AsyncRuntime as AsyncRuntime>::Instant::now();

        tracing::debug!("try to trigger election by tick, now: {:?}", now);

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

            let current_vote = self.engine.state.vote_ref();
            let utime = self.engine.state.vote_last_modified();
            let timer_config = &self.engine.config.timer_config;

            let mut election_timeout = if current_vote.is_committed() {
                timer_config.leader_lease + timer_config.election_timeout
            } else {
                timer_config.election_timeout
            };

            if self.engine.is_there_greater_log() {
                election_timeout += timer_config.smaller_log_timeout;
            }

            tracing::debug!(
                "vote utime: {:?}, current_vote: {}, now-utime:{:?}, election_timeout: {:?}",
                utime,
                current_vote,
                utime.map(|x| now - x),
                election_timeout,
            );

            // Follower/Candidate timer: next election
            if utime > Some(now - election_timeout) {
                tracing::debug!("election timeout has not yet passed",);
                return;
            }

            tracing::info!("election timeout passed, check if it is a voter for election");
        }

        // Every time elect, reset this flag.
        self.engine.reset_greater_log();

        tracing::info!("do trigger election");
        self.engine.elect();
    }

    #[tracing::instrument(level = "debug", skip_all)]
    fn handle_replication_progress(
        &mut self,
        target: C::NodeId,
        id: u64,
        result: Result<UTime<ReplicationResult<C::NodeId>, InstantOf<C>>, String>,
    ) {
        tracing::debug!(
            target = display(target),
            result = debug(&result),
            "handle_replication_progress"
        );

        if tracing::enabled!(Level::DEBUG) {
            if let Some(l) = &self.leader_data {
                if !l.replications.contains_key(&target) {
                    tracing::warn!("leader has removed target: {}", target);
                };
            } else {
                // TODO: A leader may have stepped down.
            }
        }

        // TODO: A leader may have stepped down.
        if self.engine.internal_server_state.is_leading() {
            self.engine.replication_handler().update_progress(target, id, result);
        }
    }

    /// If a message is sent by a previous server state but is received by current server state,
    /// it is a stale message and should be just ignored.
    fn does_vote_match(&self, vote: &Vote<C::NodeId>, msg: impl Display) -> bool {
        if vote != self.engine.state.vote_ref() {
            tracing::warn!(
                "vote changed: msg sent by: {:?}; curr: {}; ignore when ({})",
                vote,
                self.engine.state.vote_ref(),
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
        session_id: &ReplicationSessionId<C::NodeId>,
        msg: impl Display + Copy,
    ) -> bool {
        if !self.does_vote_match(&session_id.vote, msg) {
            return false;
        }

        if &session_id.membership_log_id != self.engine.state.membership_state.effective().log_id() {
            tracing::warn!(
                "membership_log_id changed: msg sent by: {}; curr: {}; ignore when ({})",
                session_id.membership_log_id.summary(),
                self.engine.state.membership_state.effective().log_id().summary(),
                msg
            );
            return false;
        }
        true
    }
}

#[add_async_trait]
impl<C, N, LS, SM> RaftRuntime<C> for RaftCore<C, N, LS, SM>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    async fn run_command<'e>(&mut self, cmd: Command<C>) -> Result<Option<Command<C>>, StorageError<C::NodeId>> {
        let condition = cmd.condition();
        tracing::debug!("condition: {:?}", condition);

        if let Some(condition) = condition {
            match condition {
                Condition::LogFlushed { .. } => {
                    todo!()
                }
                Condition::Applied { log_id } => {
                    if self.engine.state.io_applied() < log_id.as_ref() {
                        tracing::debug!(
                            "log_id: {} has not yet applied, postpone cmd: {:?}",
                            DisplayOption(log_id),
                            cmd
                        );
                        return Ok(Some(cmd));
                    }
                }
                Condition::StateMachineCommand { command_seq } => {
                    if self.command_state.finished_sm_seq < *command_seq {
                        tracing::debug!(
                            "sm::Command({}) has not yet finished({}), postpone cmd: {:?}",
                            command_seq,
                            self.command_state.finished_sm_seq,
                            cmd
                        );
                        return Ok(Some(cmd));
                    }
                }
            }
        }

        match cmd {
            Command::BecomeLeader => {
                debug_assert!(self.leader_data.is_none(), "can not become leader twice");
                self.leader_data = Some(LeaderData::new());
            }
            Command::QuitLeader => {
                self.leader_data = None;
            }
            Command::AppendEntry { entry } => {
                let log_id = *entry.get_log_id();
                tracing::debug!("AppendEntry: {}", &entry);

                self.append_to_log([entry], log_id).await?;

                // The leader may have changed.
                // But reporting to a different leader is not a problem.
                if let Ok(mut lh) = self.engine.leader_handler() {
                    lh.replication_handler().update_local_progress(Some(log_id));
                }
            }
            Command::AppendInputEntries { entries } => {
                let last_log_id = *entries.last().unwrap().get_log_id();
                tracing::debug!("AppendInputEntries: {}", DisplaySlice::<_>(&entries),);

                self.append_to_log(entries, last_log_id).await?;

                // The leader may have changed.
                // But reporting to a different leader is not a problem.
                if let Ok(mut lh) = self.engine.leader_handler() {
                    lh.replication_handler().update_local_progress(Some(last_log_id));
                }
            }
            Command::SaveVote { vote } => {
                self.log_store.save_vote(&vote).await?;
                self.engine.state.io_state_mut().update_vote(vote);
            }
            Command::PurgeLog { upto } => {
                self.log_store.purge(upto).await?;
                self.engine.state.io_state_mut().update_purged(Some(upto));
            }
            Command::DeleteConflictLog { since } => {
                self.log_store.truncate(since).await?;

                // Inform clients waiting for logs to be applied.
                let removed = self.client_resp_channels.split_off(&since.index);
                if !removed.is_empty() {
                    let leader_id = self.current_leader();
                    let leader_node = self.get_leader_node(leader_id);

                    // False positive lint warning(`non-binding `let` on a future`): https://github.com/rust-lang/rust-clippy/issues/9932
                    #[allow(clippy::let_underscore_future)]
                    let _ = AsyncRuntimeOf::<C>::spawn(async move {
                        for (log_index, tx) in removed.into_iter() {
                            let res = tx.send(Err(ClientWriteError::ForwardToLeader(ForwardToLeader {
                                leader_id,
                                leader_node: leader_node.clone(),
                            })));

                            tracing::debug!(
                                "sent ForwardToLeader for log_index: {}, is_ok: {}",
                                log_index,
                                res.is_ok()
                            );
                        }
                    });
                }
            }
            Command::SendVote { vote_req } => {
                self.spawn_parallel_vote_requests(&vote_req).await;
            }
            Command::ReplicateCommitted { committed } => {
                if let Some(l) = &self.leader_data {
                    for node in l.replications.values() {
                        let _ = node.tx_repl.send(Replicate::Committed(committed));
                    }
                } else {
                    unreachable!("it has to be a leader!!!");
                }
            }
            Command::Commit {
                seq,
                ref already_committed,
                ref upto,
            } => {
                self.log_store.save_committed(Some(*upto)).await?;
                self.apply_to_state_machine(seq, already_committed.next_index(), upto.index).await?;
            }
            Command::Replicate { req, target } => {
                if let Some(l) = &self.leader_data {
                    let node = l.replications.get(&target).expect("replication to target node exists");

                    match req {
                        Inflight::None => {
                            let _ = node.tx_repl.send(Replicate::Heartbeat);
                        }
                        Inflight::Logs { id, log_id_range } => {
                            let _ = node.tx_repl.send(Replicate::logs(Some(id), log_id_range));
                        }
                        Inflight::Snapshot { id, last_log_id } => {
                            let _ = last_log_id;

                            // Create a channel to let state machine worker to send the snapshot and the replication
                            // worker to receive it.
                            let (tx, rx) = oneshot::channel();

                            let cmd = sm::Command::get_snapshot(tx);
                            self.sm_handle
                                .send(cmd)
                                .map_err(|e| StorageIOError::read_snapshot(None, AnyError::error(e)))?;

                            // unwrap: The replication channel must not be dropped or it is a bug.
                            node.tx_repl.send(Replicate::snapshot(Some(id), rx)).map_err(|_e| {
                                StorageIOError::read_snapshot(None, AnyError::error("replication channel closed"))
                            })?;
                        }
                    }
                } else {
                    unreachable!("it has to be a leader!!!");
                }
            }
            Command::RebuildReplicationStreams { targets } => {
                self.remove_all_replication().await;

                for (target, matching) in targets.iter() {
                    let handle = self.spawn_replication_stream(*target, *matching).await;

                    if let Some(l) = &mut self.leader_data {
                        l.replications.insert(*target, handle);
                    } else {
                        unreachable!("it has to be a leader!!!");
                    }
                }
            }
            Command::StateMachine { command } => {
                // Just forward a state machine command to the worker.
                self.sm_handle.send(command).map_err(|_e| {
                    StorageIOError::write_state_machine(AnyError::error("can not send to sm::Worker".to_string()))
                })?;
            }
            Command::Respond { resp: send, .. } => {
                send.send();
            }
        }

        Ok(None)
    }
}
