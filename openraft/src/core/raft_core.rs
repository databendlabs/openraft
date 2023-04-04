use std::collections::BTreeMap;
use std::fmt::Display;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use futures::future::abortable;
use futures::future::select;
use futures::future::Either;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use futures::TryFutureExt;
use maplit::btreemap;
use maplit::btreeset;
use pin_utils::pin_mut;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::time::timeout;
use tokio::time::Duration;
use tokio::time::Instant;
use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::config::Config;
use crate::config::RuntimeConfig;
use crate::config::SnapshotPolicy;
use crate::core::building_state;
use crate::core::snapshot_state;
use crate::core::ServerState;
use crate::core::SnapshotResult;
use crate::display_ext::DisplaySlice;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::SendResult;
use crate::entry::FromAppData;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::error::CheckIsLeaderError;
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
use crate::metrics::UpdateMatchedLogId;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::quorum::QuorumSet;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::AppendEntriesTx;
use crate::raft::ClientWriteResponse;
use crate::raft::ClientWriteTx;
use crate::raft::ExternalCommand;
use crate::raft::RaftMsg;
use crate::raft::RaftRespTx;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::VoteTx;
use crate::raft_state::LogStateReader;
use crate::replication::Replicate;
use crate::replication::ReplicationCore;
use crate::replication::ReplicationHandle;
use crate::replication::ReplicationResult;
use crate::replication::ReplicationSessionId;
use crate::runtime::RaftRuntime;
use crate::storage::RaftSnapshotBuilder;
use crate::versioned::Updatable;
use crate::versioned::Versioned;
use crate::ChangeMembers;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::RPCTypes;
use crate::RaftNetwork;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::SnapshotId;
use crate::StorageError;
use crate::StorageHelper;
use crate::Update;
use crate::Vote;

/// Data for a Leader.
///
/// It is created when RaftCore enters leader state, and will be dropped when it quits leader state.
pub(crate) struct LeaderData<C: RaftTypeConfig, SD>
where SD: AsyncRead + AsyncSeek + Send + Unpin + 'static
{
    /// Channels to send result back to client when logs are committed.
    pub(crate) client_resp_channels: BTreeMap<u64, ClientWriteTx<C>>,

    /// A mapping of node IDs the replication state of the target node.
    // TODO(xp): make it a field of RaftCore. it does not have to belong to leader.
    //           It requires the Engine to emit correct add/remove replication commands
    pub(super) replications: BTreeMap<C::NodeId, ReplicationHandle<C::NodeId, C::Node, SD>>,

    /// The metrics of all replication streams
    pub(crate) replication_metrics: Versioned<ReplicationMetrics<C::NodeId>>,

    /// The time to send next heartbeat.
    pub(crate) next_heartbeat: Instant,
}

impl<C: RaftTypeConfig, SD> LeaderData<C, SD>
where SD: AsyncRead + AsyncSeek + Send + Unpin + 'static
{
    pub(crate) fn new() -> Self {
        Self {
            client_resp_channels: Default::default(),
            replications: BTreeMap::new(),
            replication_metrics: Versioned::new(ReplicationMetrics::default()),
            next_heartbeat: Instant::now(),
        }
    }
}

/// The core type implementing the Raft protocol.
pub struct RaftCore<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    /// This node's ID.
    pub(crate) id: C::NodeId,

    /// This node's runtime config.
    pub(crate) config: Arc<Config>,

    pub(crate) runtime_config: Arc<RuntimeConfig>,

    /// The `RaftNetworkFactory` implementation.
    pub(crate) network: N,

    /// The `RaftStorage` implementation.
    pub(crate) storage: S,

    pub(crate) engine: Engine<C::NodeId, C::Node, C::Entry>,

    pub(crate) leader_data: Option<LeaderData<C, S::SnapshotData>>,

    /// The node's current snapshot state.
    pub(crate) snapshot_state: snapshot_state::State<C, S::SnapshotData>,

    /// Received snapshot that are ready to install.
    pub(crate) received_snapshot: BTreeMap<SnapshotId, Box<S::SnapshotData>>,

    pub(crate) tx_api: mpsc::UnboundedSender<RaftMsg<C, N, S>>,
    pub(crate) rx_api: mpsc::UnboundedReceiver<RaftMsg<C, N, S>>,

    pub(crate) tx_metrics: watch::Sender<RaftMetrics<C::NodeId, C::Node>>,

    pub(crate) span: Span,
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    /// The main loop of the Raft protocol.
    pub(crate) async fn main(mut self, rx_shutdown: oneshot::Receiver<()>) -> Result<(), Fatal<C::NodeId>> {
        let span = tracing::span!(parent: &self.span, Level::DEBUG, "main");
        let res = self.do_main(rx_shutdown).instrument(span).await;

        // Flush buffered metrics
        self.report_metrics(Update::AsIs);

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

        let now = Instant::now();
        self.engine.timer.update_now(now);

        self.engine.startup();
        // It may not finish running all of the commands, if there is a command waiting for a callback.
        // TODO: this can be moved into runtime_loop()
        self.run_engine_commands().await?;

        // Initialize metrics.
        self.report_metrics(Update::Update(None));

        self.runtime_loop(rx_shutdown).await
    }

    /// Handle `is_leader` requests.
    ///
    /// Spawn requests to all members of the cluster, include members being added in joint
    /// consensus. Each request will have a timeout, and we respond once we have a majority
    /// agreement from each config group. Most of the time, we will have a single uniform
    /// config group.
    ///
    /// From the spec (§8):
    /// Second, a leader must check whether it has been deposed before processing a read-only
    /// request (its information may be stale if a more recent leader has been elected). Raft
    /// handles this by having the leader exchange heartbeat messages with a majority of the
    /// cluster before responding to read-only requests.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    pub(super) async fn handle_check_is_leader_request(
        &mut self,
        tx: RaftRespTx<(), CheckIsLeaderError<C::NodeId, C::Node>>,
    ) -> Result<(), StorageError<C::NodeId>> {
        // Setup sentinel values to track when we've received majority confirmation of leadership.

        let em = self.engine.state.membership_state.effective();
        let mut granted = btreeset! {self.id};

        if em.is_quorum(granted.iter()) {
            let _ = tx.send(Ok(()));
            return Ok(());
        }

        // Spawn parallel requests, all with the standard timeout for heartbeats.
        let mut pending = FuturesUnordered::new();

        let voter_progresses = {
            let l = &self.engine.internal_server_state.leading().unwrap();
            l.progress
                .iter()
                .filter(|(id, _v)| l.progress.is_voter(id) == Some(true))
                .copied()
                .collect::<Vec<_>>()
        };

        for (target, progress) in voter_progresses {
            if target == self.id {
                continue;
            }

            let rpc = AppendEntriesRequest {
                vote: *self.engine.state.vote_ref(),
                prev_log_id: progress.matching,
                entries: vec![],
                leader_commit: self.engine.state.committed().copied(),
            };

            let my_id = self.id;
            // Safe unwrap(): target is in membership
            let target_node = self.engine.state.membership_state.effective().get_node(&target).unwrap().clone();
            let mut client = self.network.new_client(target, &target_node).await;

            let ttl = Duration::from_millis(self.config.heartbeat_interval);

            let task = tokio::spawn(
                async move {
                    let outer_res = timeout(ttl, client.send_append_entries(rpc)).await;
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
                }
                // TODO(xp): add target to span
                .instrument(tracing::debug_span!("SPAWN_append_entries")),
            )
            .map_err(move |err| (target, err));

            pending.push(task);
        }

        // Handle responses as they return.
        while let Some(res) = pending.next().await {
            let (target, data) = match res {
                Ok(Ok(res)) => res,
                Ok(Err((target, err))) => {
                    tracing::error!(target=display(target), error=%err, "timeout while confirming leadership for read request");
                    continue;
                }
                Err((target, err)) => {
                    tracing::error!(target = display(target), "{}", err);
                    continue;
                }
            };

            // If we receive a response with a greater term, then revert to follower and abort this
            // request.
            if let AppendEntriesResponse::HigherVote(vote) = data {
                debug_assert!(
                    &vote > self.engine.state.vote_ref(),
                    "committed vote({}) has total order relation with other votes({})",
                    self.engine.state.vote_ref(),
                    vote
                );

                let res = self.engine.vote_handler().handle_message_vote(&vote);
                self.run_engine_commands().await?;

                if let Err(e) = res {
                    // simply ignore stale responses
                    tracing::warn!(target = display(target), "vote {vote} rejected: {e}");
                    continue;
                }
                // we are no longer leader so error out early
                if !self.engine.state.is_leader(&self.engine.config.id) {
                    self.reject_with_forward_to_leader(tx);
                    return Ok(());
                }
            }

            granted.insert(target);

            let mem = &self.engine.state.membership_state.effective();
            if mem.is_quorum(granted.iter()) {
                let _ = tx.send(Ok(()));
                return Ok(());
            }
        }

        // If we've hit this location, then we've failed to gather needed confirmations due to
        // request failures.

        let _ = tx.send(Err(QuorumNotEnough {
            cluster: self.engine.state.membership_state.effective().membership().summary(),
            got: granted,
        }
        .into()));
        Ok(())
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
        tx: RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId, C::Node>>,
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
            if let Some(l) = &mut self.leader_data {
                l.client_resp_channels.insert(index, tx);
            }
        }

        true
    }

    /// Send a heartbeat message to every followers/learners.
    ///
    /// Currently heartbeat is a blank log
    #[tracing::instrument(level = "debug", skip_all, fields(id = display(self.id)))]
    pub fn send_heartbeat(&mut self, emitter: impl Display) -> bool {
        tracing::debug!(now = debug(self.engine.timer.now()), "send_heartbeat");

        let mut lh = if let Some((lh, _)) =
            self.engine.get_leader_handler_or_reject::<(), ClientWriteError<C::NodeId, C::Node>>(None)
        {
            lh
        } else {
            tracing::debug!(
                now = debug(self.engine.timer.now()),
                "{} failed to send heartbeat",
                emitter
            );
            return false;
        };

        lh.send_heartbeat();

        tracing::debug!("{} triggered sending heartbeat", emitter);
        true
    }

    /// Flush cached changes of metrics to notify metrics watchers with updated metrics.
    /// Then clear flags about the cached changes, to avoid unnecessary metrics report.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn flush_metrics(&mut self) {
        if !self.engine.output.metrics_flags.changed() {
            return;
        }

        let leader_metrics = if self.engine.output.metrics_flags.replication {
            let replication_metrics = self.leader_data.as_ref().map(|x| x.replication_metrics.clone());
            Update::Update(replication_metrics)
        } else {
            #[allow(clippy::collapsible_else_if)]
            if self.leader_data.is_some() {
                Update::AsIs
            } else {
                Update::Update(None)
            }
        };

        self.report_metrics(leader_metrics);
        self.engine.output.metrics_flags.reset();
    }

    /// Report a metrics payload on the current state of the Raft node.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn report_metrics(&self, replication: Update<Option<Versioned<ReplicationMetrics<C::NodeId>>>>) {
        let replication = match replication {
            Update::Update(v) => v,
            Update::AsIs => self.tx_metrics.borrow().replication.clone(),
        };

        let m = RaftMetrics {
            running_state: Ok(()),
            id: self.id,

            // --- data ---
            current_term: self.engine.state.vote_ref().leader_id().get_term(),
            last_log_index: self.engine.state.last_log_id().index(),
            last_applied: self.engine.state.committed().copied(),
            snapshot: self.engine.state.snapshot_meta.last_log_id,

            // --- cluster ---
            state: self.engine.state.server_state,
            current_leader: self.current_leader(),
            membership_config: self.engine.state.membership_state.effective().stored_membership().clone(),

            // --- replication ---
            replication,
        };

        {
            let curr = self.tx_metrics.borrow();
            if m == *curr {
                tracing::debug!("metrics not changed: {}", m.summary());
                return;
            }
        }

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
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn handle_initialize(
        &mut self,
        member_nodes: BTreeMap<C::NodeId, C::Node>,
        tx: RaftRespTx<(), InitializeError<C::NodeId, C::Node>>,
    ) {
        let membership = Membership::from(member_nodes);

        let entry = C::Entry::new_membership(LogId::default(), membership);
        let res = self.engine.initialize(entry);
        self.engine.output.push_command(Command::SendInitializeResult {
            send: SendResult::new(res, tx),
        });
    }

    fn handle_building_snapshot_result(
        &mut self,
        result: SnapshotResult<C::NodeId, C::Node>,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("handle_building_snapshot_result: {:?}", result);

        debug_assert!(self.snapshot_state.building.is_some(), "no snapshot is building");

        // One build will only be triggered if there is no other build in progress.
        // Therefore there won't be multiple builds in progress.
        match result {
            SnapshotResult::Ok(meta) => {
                self.engine.finish_building_snapshot(meta);
            }
            SnapshotResult::StorageError(sto_err) => {
                return Err(sto_err);
            }
            SnapshotResult::Aborted => {}
        }

        self.snapshot_state.building = None;

        Ok(())
    }

    /// Trigger a snapshot building(log compaction) job if needed.
    ///
    /// If `force` is True, it will skip the threshold check and start creating snapshot as
    /// demanded. But it will still check if there is already a snapshot building job in progress.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) async fn trigger_snapshot_if_needed(&mut self, force: bool) {
        tracing::debug!("trigger_snapshot_if_needed: force: {}", force);

        if self.snapshot_state.building.is_some() {
            tracing::debug!("snapshot building is in progress, do not trigger snapshot");
            return;
        }

        let SnapshotPolicy::LogsSinceLast(threshold) = self.config.snapshot_policy;

        if !force {
            // If we are below the threshold, then there is nothing to do.
            if self.engine.state.committed().next_index() - self.engine.state.snapshot_meta.last_log_id.next_index()
                < threshold
            {
                return;
            }
        }

        // At this point, we are clear to begin a new snapshot building process.

        let mut builder = self.storage.get_snapshot_builder().await;
        let (fu, abort_handle) = abortable(async move { builder.build_snapshot().await });
        let tx_api = self.tx_api.clone();

        let join_handle = tokio::spawn(
            async move {
                match fu.await {
                    Ok(res) => match res {
                        Ok(snapshot) => {
                            let _ = tx_api.send(RaftMsg::BuildingSnapshotResult {
                                result: SnapshotResult::Ok(snapshot.meta),
                            });
                        }
                        Err(err) => {
                            tracing::error!({error=%err}, "error while generating snapshot");
                            let _ = tx_api.send(RaftMsg::BuildingSnapshotResult {
                                result: SnapshotResult::StorageError(err),
                            });
                        }
                    },
                    Err(_aborted) => {
                        let _ = tx_api.send(RaftMsg::BuildingSnapshotResult {
                            result: SnapshotResult::Aborted,
                        });
                    }
                }
            }
            .instrument(tracing::debug_span!("building-snapshot")),
        );

        self.snapshot_state.building = Some(building_state::Building {
            abort_handle,
            join_handle,
        });
    }

    /// Reject a request due to the Raft node being in a state which prohibits the request.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    pub(crate) fn reject_with_forward_to_leader<T, E>(&self, tx: RaftRespTx<T, E>)
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
    pub fn current_leader(&self) -> Option<C::NodeId> {
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

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn apply_to_state_machine(
        &mut self,
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

        let entries = StorageHelper::new(&mut self.storage).get_log_entries(since..end).await?;
        tracing::debug!(
            entries = display(DisplaySlice::<_>(entries.as_slice())),
            "about to apply"
        );

        // TODO: prepare response before apply_to_state_machine,
        //       so that an Entry does not need to be Clone,
        //       and no references will be used by apply_to_state_machine
        let apply_results = self.storage.apply_to_state_machine(&entries).await?;

        let last_applied = entries[entries.len() - 1].get_log_id();
        tracing::debug!(last_applied = display(last_applied), "update last_applied");

        if let Some(l) = &mut self.leader_data {
            let mut results = apply_results.into_iter();

            for log_index in since..end {
                let tx = l.client_resp_channels.remove(&log_index);

                let i = log_index - since;
                let entry = &entries[i as usize];
                let apply_res = results.next().unwrap();

                Self::send_response(entry, apply_res, tx);
            }
        }

        self.trigger_snapshot_if_needed(false).await;
        Ok(())
    }

    /// Send result of applying a log entry to its client.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn send_response(entry: &C::Entry, resp: C::R, tx: Option<ClientWriteTx<C>>) {
        tracing::debug!(entry = display(entry), "send_response");

        let tx = match tx {
            None => return,
            Some(x) => x,
        };

        let membership = entry.get_membership().cloned();

        let res = Ok(ClientWriteResponse {
            log_id: *entry.get_log_id(),
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
    ) -> ReplicationHandle<C::NodeId, C::Node, S::SnapshotData> {
        // Safe unwrap(): target must be in membership
        let target_node = self.engine.state.membership_state.effective().get_node(&target).unwrap();

        let membership_log_id = self.engine.state.membership_state.effective().log_id();
        let network = self.network.new_client(target, target_node).await;

        let session_id = ReplicationSessionId::new(*self.engine.state.vote_ref(), *membership_log_id);

        ReplicationCore::<C, N, S>::spawn(
            target,
            session_id,
            self.config.clone(),
            self.engine.state.committed().copied(),
            progress_entry.matching,
            network,
            self.storage.get_log_reader().await,
            self.tx_api.clone(),
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

            l.replication_metrics = Versioned::new(ReplicationMetrics::default());
        } else {
            unreachable!("it has to be a leader!!!");
        };
    }
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
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
            self.run_command(cmd).await?;
        }

        Ok(())
    }

    /// Run an event handling loop
    #[tracing::instrument(level="debug", skip_all, fields(id=display(self.id)))]
    async fn runtime_loop(&mut self, mut rx_shutdown: oneshot::Receiver<()>) -> Result<(), Fatal<C::NodeId>> {
        loop {
            self.flush_metrics();

            let msg_res: Result<RaftMsg<C, N, S>, &str> = {
                let recv = self.rx_api.recv();
                pin_mut!(recv);

                let either = select(recv, Pin::new(&mut rx_shutdown)).await;

                match either {
                    Either::Left((recv_res, _shutdown)) => match recv_res {
                        Some(msg) => Ok(msg),
                        None => Err("all rx_api senders are dropped"),
                    },
                    Either::Right((_rx_shutdown_res, _recv)) => Err("recv from rx_shutdown"),
                }
            };

            match msg_res {
                Ok(msg) => {
                    self.handle_api_msg(msg).await?;

                    // Run as many commands as possible.
                    // If there is a command that waits for a callback, just return and wait for next message.
                    self.run_engine_commands().await?;
                }
                Err(reason) => {
                    tracing::info!(reason);
                    return Ok(());
                }
            }
        }
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

            let tx = self.tx_api.clone();

            let ttl = Duration::from_millis(self.config.election_timeout_min);
            let id = self.id;

            let _ = tokio::spawn(
                async move {
                    let tm_res = timeout(ttl, client.send_vote(req)).await;
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
                            let _ = tx.send(RaftMsg::VoteResponse { target, resp, vote });
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
        tracing::debug!(req = display(req.summary()), func = func_name!());

        let resp = self.engine.handle_vote_req(req);
        self.engine.output.push_command(Command::SendVoteResult {
            send: SendResult::new(Ok(resp), tx),
        });
    }

    /// Handle response from a vote request sent to a peer.
    #[tracing::instrument(level = "debug", skip_all)]
    fn handle_vote_resp(&mut self, resp: VoteResponse<C::NodeId>, target: C::NodeId) {
        self.engine.handle_vote_resp(target, resp);
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
    pub(crate) async fn handle_api_msg(&mut self, msg: RaftMsg<C, N, S>) -> Result<(), Fatal<C::NodeId>> {
        tracing::debug!("recv from rx_api: {}", msg.summary());

        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                self.handle_append_entries_request(rpc, tx);
            }
            RaftMsg::RequestVote { rpc, tx } => {
                // Vote request needs to check if the lease of the last leader expired.
                // Thus it is time sensitive. Update the cached time for it.
                let now = Instant::now();
                self.engine.timer.update_now(now);
                tracing::debug!(
                    vote_request = display(rpc.summary()),
                    "handle vote request: now: {:?}",
                    now
                );

                self.handle_vote_request(rpc, tx);
            }
            RaftMsg::VoteResponse { target, resp, vote } => {
                let now = Instant::now();
                self.engine.timer.update_now(now);

                if self.does_vote_match(&vote, "VoteResponse") {
                    self.handle_vote_resp(resp, target);
                }
            }
            RaftMsg::InstallSnapshot { rpc, tx } => {
                self.handle_install_snapshot_request(rpc, tx).await?;
            }
            RaftMsg::BuildingSnapshotResult { result } => {
                self.handle_building_snapshot_result(result)?;
            }
            RaftMsg::CheckIsLeaderRequest { tx } => {
                if self.engine.state.is_leader(&self.engine.config.id) {
                    self.handle_check_is_leader_request(tx).await?;
                } else {
                    self.reject_with_forward_to_leader(tx);
                }
            }
            RaftMsg::ClientWriteRequest { app_data, tx } => {
                self.write_entry(C::Entry::from_app_data(app_data), Some(tx));
            }
            RaftMsg::Initialize { members, tx } => {
                self.handle_initialize(members, tx);
            }
            RaftMsg::AddLearner { id, node, tx } => {
                self.change_membership(ChangeMembers::AddNodes(btreemap! {id=>node}), true, tx);
            }
            RaftMsg::ChangeMembership { changes, retain, tx } => {
                self.change_membership(changes, retain, tx);
            }
            RaftMsg::ExternalRequest { req } => {
                req(&self.engine.state, &mut self.storage, &mut self.network);
            }
            RaftMsg::ExternalCommand { cmd } => {
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
                    ExternalCommand::Snapshot => self.trigger_snapshot_if_needed(true).await,
                }
            }
            RaftMsg::Tick { i } => {
                // check every timer

                let now = Instant::now();
                // TODO: store server start time and use relative time
                self.engine.timer.update_now(now);
                tracing::debug!("received tick: {}, now: {:?}", i, now);

                self.handle_tick_election();

                // TODO: test: with heartbeat log, election is automatically rejected.
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
                            l.next_heartbeat = Instant::now() + Duration::from_millis(self.config.heartbeat_interval);
                        }
                    }
                }

                // When a membership that removes the leader is committed,
                // the leader continue to work for a short while before reverting to a learner.
                // This way, let the leader replicate the `membership-log-is-committed` message to
                // followers. Otherwise, if the leader step down at once, the
                // follower might have to re-commit the membership log
                // again, electing itself.
                self.engine.leader_step_down();
            }

            RaftMsg::HigherVote {
                target: _,
                higher,
                vote,
            } => {
                if self.does_vote_match(&vote, "HigherVote") {
                    // Rejected vote change is ok.
                    let _ = self.engine.vote_handler().handle_message_vote(&higher);
                }
            }

            RaftMsg::UpdateReplicationProgress {
                target,
                id,
                result,
                session_id,
            } => {
                // If vote or membership changes, ignore the message.
                // There is chance delayed message reports a wrong state.
                if self.does_replication_session_match(&session_id, "UpdateReplicationMatched") {
                    self.handle_replication_progress(target, id, result);
                }
            }
            RaftMsg::ReplicationFatal => {
                return Err(Fatal::Stopped);
            }
        };
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    fn handle_tick_election(&mut self) {
        let now = *self.engine.timer.now();

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
        result: Result<ReplicationResult<C::NodeId>, String>,
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

    #[tracing::instrument(level = "debug", skip_all)]
    fn update_progress_metrics(&mut self, target: C::NodeId, matching: LogId<C::NodeId>) {
        tracing::debug!(%target, ?matching, "update_leader_metrics");

        if let Some(l) = &mut self.leader_data {
            tracing::debug!(
                target = display(target),
                matching = debug(&matching),
                "update replication_metrics"
            );
            l.replication_metrics.update(UpdateMatchedLogId { target, matching });
        } else {
            // This method is only called after `update_progress()`.
            // And this node may become a non-leader after `update_progress()`
        }

        self.engine.output.metrics_flags.set_replication_changed()
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

#[async_trait::async_trait]
impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftRuntime<C> for RaftCore<C, N, S> {
    async fn run_command<'e>(
        &mut self,
        cmd: Command<C::NodeId, C::Node, C::Entry>,
    ) -> Result<(), StorageError<C::NodeId>> {
        match cmd {
            Command::BecomeLeader => {
                debug_assert!(self.leader_data.is_none(), "can not become leader twice");
                self.leader_data = Some(LeaderData::new());
            }
            Command::QuitLeader => {
                if let Some(l) = &mut self.leader_data {
                    // Leadership lost, inform waiting clients
                    let chans = std::mem::take(&mut l.client_resp_channels);
                    for (_, tx) in chans.into_iter() {
                        let _ = tx.send(Err(ClientWriteError::ForwardToLeader(ForwardToLeader {
                            leader_id: None,
                            leader_node: None,
                        })));
                    }
                }
                self.leader_data = None;
            }
            Command::AppendEntry { entry } => {
                tracing::debug!("AppendEntry: {}", &entry);
                self.storage.append_to_log([entry]).await?
            }
            Command::AppendInputEntries { entries } => {
                tracing::debug!("AppendInputEntries: {}", DisplaySlice::<_>(&entries),);
                self.storage.append_to_log(entries).await?
            }
            Command::AppendBlankLog { log_id } => {
                let ent = C::Entry::new_blank(log_id);
                let entries = [ent];
                self.storage.append_to_log(entries).await?
            }
            Command::SaveVote { vote } => {
                self.storage.save_vote(&vote).await?;
            }
            Command::PurgeLog { upto } => self.storage.purge_logs_upto(upto).await?,
            Command::DeleteConflictLog { since } => {
                self.storage.delete_conflict_logs_since(since).await?;
            }
            // TODO(2): Engine initiate a snapshot building
            Command::BuildSnapshot { .. } => {}
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
            Command::LeaderCommit {
                ref already_committed,
                ref upto,
            } => {
                self.apply_to_state_machine(already_committed.next_index(), upto.index).await?;
            }
            Command::FollowerCommit {
                ref already_committed,
                ref upto,
            } => {
                self.apply_to_state_machine(already_committed.next_index(), upto.index).await?;
            }
            Command::Replicate { req, target } => {
                if let Some(l) = &self.leader_data {
                    let node = l.replications.get(&target).expect("replication to target node exists");

                    match req {
                        Inflight::None => {
                            let _ = node.tx_repl.send(Replicate::Heartbeat);
                        }
                        Inflight::Logs { id, log_id_range } => {
                            let _ = node.tx_repl.send(Replicate::logs(id, log_id_range));
                        }
                        Inflight::Snapshot { id, last_log_id } => {
                            let snapshot = self.storage.get_current_snapshot().await?;
                            tracing::debug!("snapshot: {}", snapshot.as_ref().map(|x| &x.meta).summary());

                            if let Some(snapshot) = snapshot {
                                debug_assert_eq!(last_log_id, snapshot.meta.last_log_id);
                                let _ = node.tx_repl.send(Replicate::snapshot(id, snapshot));
                            } else {
                                unreachable!("No snapshot");
                            }
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
            Command::UpdateProgressMetrics { target, matching } => {
                self.update_progress_metrics(target, matching);
            }
            Command::UpdateMembership { .. } => {
                // TODO: not used
            }
            Command::CancelSnapshot { snapshot_meta } => {
                let got = self.received_snapshot.remove(&snapshot_meta.snapshot_id);
                debug_assert!(got.is_some(), "there has to be a buffered snapshot data");
            }
            Command::InstallSnapshot { snapshot_meta } => {
                let snapshot_data = self.received_snapshot.remove(&snapshot_meta.snapshot_id);

                if let Some(data) = snapshot_data {
                    self.storage.install_snapshot(&snapshot_meta, data).await?;
                    tracing::debug!("Done install_snapshot, meta: {:?}", snapshot_meta);
                } else {
                    unreachable!("buffered snapshot not found: snapshot meta: {:?}", snapshot_meta)
                }
            }
            Command::SendVoteResult { send } => {
                send.send();
            }
            Command::SendAppendEntriesResult { send } => {
                send.send();
            }
            Command::SendInstallSnapshotResult { send } => {
                send.send();
            }
            Command::SendInitializeResult { send } => {
                send.send();
            }
        }

        Ok(())
    }
}
