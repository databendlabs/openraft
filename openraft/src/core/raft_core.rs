use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Display;
use std::mem::swap;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use futures::future::abortable;
use futures::future::select;
use futures::future::Either;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use futures::TryFutureExt;
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
use crate::core::replication_lag;
use crate::core::Expectation;
use crate::core::ServerState;
use crate::core::SnapshotResult;
use crate::core::SnapshotState;
use crate::core::VoteWiseTime;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::SendResult;
use crate::entry::EntryRef;
use crate::error::ChangeMembershipError;
use crate::error::CheckIsLeaderError;
use crate::error::ClientWriteError;
use crate::error::EmptyMembership;
use crate::error::Fatal;
use crate::error::ForwardToLeader;
use crate::error::InProgress;
use crate::error::InitializeError;
use crate::error::LearnerIsLagging;
use crate::error::LearnerNotFound;
use crate::error::QuorumNotEnough;
use crate::error::RPCError;
use crate::error::Timeout;
use crate::metrics::RaftMetrics;
use crate::metrics::ReplicationMetrics;
use crate::metrics::UpdateMatchedLogId;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::quorum::QuorumSet;
use crate::raft::AddLearnerResponse;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::AppendEntriesTx;
use crate::raft::ClientWriteResponse;
use crate::raft::ClientWriteTx;
use crate::raft::ExternalCommand;
use crate::raft::RaftAddLearnerTx;
use crate::raft::RaftMsg;
use crate::raft::RaftRespTx;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::VoteTx;
use crate::raft_state::LogStateReader;
use crate::raft_state::VoteStateReader;
use crate::raft_types::LogIdOptionExt;
use crate::raft_types::RaftLogId;
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
use crate::Entry;
use crate::EntryPayload;
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
use crate::Update;
use crate::Vote;

/// Data for a Leader.
///
/// It is created when RaftCore enters leader state, and will be dropped when it quits leader state.
pub(crate) struct LeaderData<C: RaftTypeConfig, SD>
where SD: AsyncRead + AsyncSeek + Send + Unpin + 'static
{
    /// Channels to send result back to client when logs are committed.
    pub(crate) client_resp_channels: BTreeMap<u64, ClientWriteTx<C, C::NodeId, C::Node>>,

    /// A mapping of node IDs the replication state of the target node.
    // TODO(xp): make it a field of RaftCore. it does not have to belong to leader.
    //           It requires the Engine to emit correct add/remove replication commands
    pub(super) nodes: BTreeMap<C::NodeId, ReplicationHandle<C::NodeId, C::Node, SD>>,

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
            nodes: BTreeMap::new(),
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

    pub(crate) engine: Engine<C::NodeId, C::Node>,

    pub(crate) leader_data: Option<LeaderData<C, S::SnapshotData>>,

    /// The node's current snapshot state.
    pub(crate) snapshot_state: SnapshotState<C, S::SnapshotData>,

    /// Received snapshot that are ready to install.
    pub(crate) received_snapshot: BTreeMap<SnapshotId, Box<S::SnapshotData>>,

    /// The time to elect if a follower does not receive any append-entry message.
    pub(crate) next_election_time: VoteWiseTime<C::NodeId>,

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

        self.engine.startup();
        self.run_engine_commands::<Entry<C>>(&[]).await?;

        // To ensure that restarted nodes don't disrupt a stable cluster.
        if self.engine.state.server_state == ServerState::Follower {
            self.set_next_election_time(false);
        }

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
                vote: *self.engine.state.get_vote(),
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
                let res = self.engine.vote_handler().handle_message_vote(&vote);
                self.run_engine_commands::<Entry<C>>(&[]).await?;

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
            cluster: self.engine.state.membership_state.effective().membership.summary(),
            got: granted,
        }
        .into()));
        Ok(())
    }

    /// Add a new node to the cluster as a learner, bringing it up-to-speed, and then responding
    /// on the given channel.
    ///
    /// Adding a learner does not affect election, thus it does not need to enter joint consensus.
    ///
    /// TODO: It has to wait for the previous membership to commit.
    /// TODO: Otherwise a second proposed membership implies the previous one is committed.
    /// TODO: Test it.
    /// TODO: This limit can be removed if membership_state is replaced by a list of membership
    /// logs. TODO: Because allowing this requires the engine to be able to store more than 2
    /// membership logs. And it does not need to wait for the previous membership log to commit
    /// to propose the new membership log.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) async fn add_learner(
        &mut self,
        target: C::NodeId,
        node: C::Node,
        tx: RaftAddLearnerTx<C::NodeId, C::Node>,
    ) -> Result<(), Fatal<C::NodeId>> {
        if let Some(l) = &self.leader_data {
            tracing::debug!(
                "add target node {} as learner; current nodes: {:?}",
                target,
                l.nodes.keys()
            );
        } else {
            unreachable!("it has to be a leader!!!");
        }

        // Ensure the node doesn't already exist in the current config,
        // in the set of new nodes already being synced, or in the nodes being removed.

        let curr = &self.engine.state.membership_state.effective();
        if curr.contains(&target) {
            let matching = if let Some(l) = &self.engine.internal_server_state.leading() {
                *l.progress.get(&target)
            } else {
                unreachable!("it has to be a leader!!!");
            };

            tracing::debug!(
                "target {:?} already member or learner, can't add; matching:{:?}",
                target,
                matching
            );

            let _ = tx.send(Ok(AddLearnerResponse {
                membership_log_id: self.engine.state.membership_state.effective().log_id,
                matched: matching.matching,
            }));
            return Ok(());
        }

        let curr = &self.engine.state.membership_state.effective().membership;
        let new_membership = curr.add_learner(target, node);

        tracing::debug!(?new_membership, "new_membership with added learner: {}", target);

        let log_id = self.write_entry(EntryPayload::Membership(new_membership), None).await?;

        tracing::debug!(
            "after add target node {} as learner; last_log_id: {:?}",
            target,
            self.engine.state.last_log_id()
        );

        let _ = tx.send(Ok(AddLearnerResponse {
            membership_log_id: Some(log_id),
            matched: None,
        }));

        Ok(())
    }

    /// Submit change-membership by writing a Membership log entry, if the `expect` is satisfied.
    ///
    /// If `turn_to_learner` is `true`, removed `voter` will becomes `learner`. Otherwise they will
    /// be just removed.
    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn change_membership(
        &mut self,
        changes: ChangeMembers<C::NodeId>,
        expectation: Option<Expectation>,
        turn_to_learner: bool,
        tx: RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId, C::Node>>,
    ) -> Result<(), Fatal<C::NodeId>> {
        let last = self.engine.state.membership_state.effective().membership.get_joint_config().last().unwrap();
        let members = changes.apply_to(last);

        // Ensure cluster will have at least one node.
        if members.is_empty() {
            let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                ChangeMembershipError::EmptyMembership(EmptyMembership {}),
            )));
            return Ok(());
        }

        let res = self.check_membership_committed();
        if let Err(e) = res {
            let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(e)));
            return Ok(());
        }

        let mem = &self.engine.state.membership_state.effective();
        let curr = mem.membership.clone();

        let old_members = mem.voter_ids().collect::<BTreeSet<_>>();
        let only_in_new = members.difference(&old_members);

        let new_config = curr.next_safe(members.clone(), turn_to_learner);

        tracing::debug!(?new_config, "new_config");

        for node_id in only_in_new.clone() {
            if !mem.contains(node_id) {
                let not_found = LearnerNotFound { node_id: *node_id };
                let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(
                    ChangeMembershipError::LearnerNotFound(not_found),
                )));
                return Ok(());
            }
        }

        if let Err(e) = self.check_replication_states(only_in_new, expectation) {
            let _ = tx.send(Err(e.into()));
            return Ok(());
        }

        self.write_entry(EntryPayload::Membership(new_config), Some(tx)).await?;
        Ok(())
    }

    /// Check if the effective membership is committed, so that a new membership is allowed to be
    /// proposed.
    fn check_membership_committed(&self) -> Result<(), ChangeMembershipError<C::NodeId>> {
        let st = &self.engine.state;

        if st.is_membership_committed() {
            return Ok(());
        }

        Err(ChangeMembershipError::InProgress(InProgress {
            committed: st.committed().copied(),
            membership_log_id: st.membership_state.effective().log_id,
        }))
    }

    /// return Ok if all the current replication states satisfy the `expectation` for changing
    /// membership.
    fn check_replication_states<'n>(
        &self,
        nodes: impl Iterator<Item = &'n C::NodeId>,
        expectation: Option<Expectation>,
    ) -> Result<(), ChangeMembershipError<C::NodeId>> {
        let expectation = match &expectation {
            None => {
                // No expectation, whatever is OK.
                return Ok(());
            }
            Some(x) => x,
        };

        let last_log_id = self.engine.state.last_log_id();

        for node_id in nodes {
            match expectation {
                Expectation::AtLineRate => {
                    // Expect to be at line rate but not.

                    let matching = if let Some(l) = &self.engine.internal_server_state.leading() {
                        *l.progress.get(node_id)
                    } else {
                        unreachable!("it has to be a leader!!!");
                    };

                    let distance = replication_lag(&matching.matching.index(), &last_log_id.index());

                    if distance <= self.config.replication_lag_threshold {
                        continue;
                    }

                    let lagging = LearnerIsLagging {
                        node_id: *node_id,
                        matched: matching.matching,
                        distance,
                    };

                    return Err(ChangeMembershipError::LearnerIsLagging(lagging));
                }
            }
        }

        Ok(())
    }

    /// Write a log entry to the cluster through raft protocol.
    ///
    /// I.e.: append the log entry to local store, forward it to a quorum(including the leader),
    /// waiting for it to be committed and applied.
    ///
    /// The result of applying it to state machine is sent to `resp_tx`, if it is not `None`.
    /// The calling side may not receive a result from `resp_tx`, if raft is shut down.
    #[tracing::instrument(level = "debug", skip_all, fields(id = display(self.id)))]
    pub async fn write_entry(
        &mut self,
        payload: EntryPayload<C>,
        resp_tx: Option<ClientWriteTx<C, C::NodeId, C::Node>>,
    ) -> Result<LogId<C::NodeId>, Fatal<C::NodeId>> {
        tracing::debug!(payload = display(payload.summary()), "write_entry");

        let mut entry_refs = [EntryRef::new(&payload)];
        // TODO: it should returns membership config error etc. currently this is done by the
        // caller.
        self.engine.leader_append_entries(&mut entry_refs);

        // Install callback channels.
        if let Some(tx) = resp_tx {
            if let Some(l) = &mut self.leader_data {
                l.client_resp_channels.insert(entry_refs[0].log_id.index, tx);
            }
        }

        self.run_engine_commands(&entry_refs).await?;

        Ok(*entry_refs[0].get_log_id())
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
            current_term: self.engine.state.get_vote().term,
            last_log_index: self.engine.state.last_log_id().index(),
            last_applied: self.engine.state.committed().copied(),
            snapshot: self.engine.state.snapshot_meta.last_log_id,

            // --- cluster ---
            state: self.engine.state.server_state,
            current_leader: self.current_leader(),
            membership_config: self.engine.state.membership_state.effective().clone(),

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
    pub(crate) async fn handle_initialize(
        &mut self,
        member_nodes: BTreeMap<C::NodeId, C::Node>,
        tx: RaftRespTx<(), InitializeError<C::NodeId, C::Node>>,
    ) -> Result<(), StorageError<C::NodeId>> {
        let membership = Membership::from(member_nodes);
        let payload = EntryPayload::<C>::Membership(membership);

        let mut entry_refs = [EntryRef::new(&payload)];
        let res = self.engine.initialize(&mut entry_refs);
        self.engine.output.push_command(Command::SendInitializeResult {
            send: SendResult::new(res, tx),
        });

        self.run_engine_commands(&entry_refs).await?;

        Ok(())
    }

    /// Set a value for the next election timeout.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn set_next_election_time(&mut self, can_be_leader: bool) {
        let now = Instant::now();

        let mut t = Duration::from_millis(self.config.new_rand_election_timeout());
        if !can_be_leader {
            t *= 2;
        }
        tracing::debug!(
            "update election timeout after: {:?}, can_be_leader: {}",
            t,
            can_be_leader
        );

        // TODO: election timer should be bound to `(vote, membership_log_id)`:
        //       i.e., when membership is updated, the previous election timer should be
        // invalidated.       e.g., in a same `vote`, a learner becomes voter and then
        // becomes learner again.             election timer should be cleared for learner,
        // set for voter and then cleared again.
        self.next_election_time = VoteWiseTime::new(*self.engine.state.get_vote(), now + t);
    }

    pub(crate) async fn handle_building_snapshot_result(
        &mut self,
        result: SnapshotResult<C::NodeId, C::Node>,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("handle_building_snapshot_result: {:?}", result);

        if let SnapshotState::Streaming { .. } = &self.snapshot_state {
            tracing::info!("snapshot is being streaming. Ignore building snapshot result");
            return Ok(());
        }

        // TODO: add building-session id to identify different building
        match result {
            SnapshotResult::Ok(meta) => {
                self.engine.finish_building_snapshot(meta);
                self.run_engine_commands::<Entry<C>>(&[]).await?;
            }
            SnapshotResult::StorageError(sto_err) => {
                return Err(sto_err);
            }
            SnapshotResult::Aborted => {}
        }

        self.snapshot_state = SnapshotState::None;

        Ok(())
    }

    /// Trigger a log compaction (snapshot) job if needed.
    /// If force is True, it will skip the threshold check and start creating snapshot as demanded.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) async fn trigger_snapshot_if_needed(&mut self, force: bool) {
        tracing::debug!("trigger_snapshot_if_needed: force: {}", force);

        if let SnapshotState::None = self.snapshot_state {
            // Continue.
        } else {
            // Snapshot building or streaming is in progress.
            return;
        }

        let SnapshotPolicy::LogsSinceLast(threshold) = &self.config.snapshot_policy;

        if !force {
            // If we are below the threshold, then there is nothing to do.
            if self.engine.state.committed().next_index() - self.engine.state.snapshot_meta.last_log_id.next_index()
                < *threshold
            {
                return;
            }
        }

        // At this point, we are clear to begin a new compaction process.
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

        self.snapshot_state = SnapshotState::Snapshotting {
            abort_handle,
            join_handle,
        };
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
            vote = display(self.engine.state.get_vote().summary()),
            "get current_leader"
        );

        if !self.engine.state.get_vote().committed {
            return None;
        }

        let id = self.engine.state.get_vote().node_id;

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

        let entries = self.storage.get_log_entries(since..end).await?;
        tracing::debug!(entries=%entries.as_slice().summary(), "about to apply");

        let entry_refs = entries.iter().collect::<Vec<_>>();
        let apply_results = self.storage.apply_to_state_machine(&entry_refs).await?;

        let last_applied = entries[entries.len() - 1].log_id;
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
    pub(super) fn send_response(entry: &Entry<C>, resp: C::R, tx: Option<ClientWriteTx<C, C::NodeId, C::Node>>) {
        tracing::debug!(entry = display(entry.summary()), "send_response");

        let tx = match tx {
            None => return,
            Some(x) => x,
        };

        let membership = if let EntryPayload::Membership(ref c) = entry.payload {
            Some(c.clone())
        } else {
            None
        };

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
    ) -> ReplicationHandle<C::NodeId, C::Node, S::SnapshotData> {
        // Safe unwrap(): target must be in membership
        let target_node = self.engine.state.membership_state.effective().get_node(&target).unwrap();

        let membership_log_id = self.engine.state.membership_state.effective().log_id;
        let network = self.network.new_client(target, target_node).await;

        let session_id = ReplicationSessionId::new(*self.engine.state.get_vote(), membership_log_id);

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
            let nodes = std::mem::take(&mut l.nodes);

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
    pub(crate) async fn run_engine_commands<'e, Ent>(
        &mut self,
        input_entries: &'e [Ent],
    ) -> Result<(), StorageError<C::NodeId>>
    where
        Ent: RaftLogId<C::NodeId> + Sync + Send + 'e,
        &'e Ent: Into<Entry<C>>,
    {
        if tracing::enabled!(Level::DEBUG) {
            tracing::debug!("commands: start...");
            for c in self.engine.output.commands.iter() {
                tracing::debug!("commands: {:?}", c);
            }
            tracing::debug!("commands: end...");
        }

        let mut curr = 0;
        let mut commands = vec![];
        swap(&mut self.engine.output.commands, &mut commands);
        for cmd in commands {
            tracing::debug!("run command: {:?}", cmd);
            self.run_command(input_entries, &mut curr, cmd).await?;
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
                Ok(msg) => self.handle_api_msg(msg).await?,
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
    pub(super) async fn handle_vote_request(
        &mut self,
        req: VoteRequest<C::NodeId>,
        tx: VoteTx<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(req = display(req.summary()), func = func_name!());

        let resp = self.engine.handle_vote_req(req);
        self.engine.output.push_command(Command::SendVoteResult {
            send: SendResult::new(Ok(resp), tx),
        });

        self.run_engine_commands::<Entry<C>>(&[]).await?;

        Ok(())
    }

    /// Handle response from a vote request sent to a peer.
    #[tracing::instrument(level = "debug", skip(self, resp))]
    async fn handle_vote_resp(
        &mut self,
        resp: VoteResponse<C::NodeId>,
        target: C::NodeId,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(
            resp = debug(&resp),
            target = display(target),
            my_vote = display(&self.engine.state.get_vote()),
            my_last_log_id = debug(self.engine.state.last_log_id()),
            "recv vote response"
        );

        self.engine.handle_vote_resp(target, resp);
        self.run_engine_commands::<Entry<C>>(&[]).await?;

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) async fn handle_append_entries_request(
        &mut self,
        req: AppendEntriesRequest<C>,
        tx: AppendEntriesTx<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(req = display(req.summary()), func = func_name!());

        let resp = self.engine.handle_append_entries_req(&req.vote, req.prev_log_id, &req.entries, req.leader_commit);
        self.engine.output.push_command(Command::SendAppendEntriesResult {
            send: SendResult::new(Ok(resp), tx),
        });

        self.run_engine_commands(req.entries.as_slice()).await?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = debug(self.engine.state.server_state), id=display(self.id)))]
    pub(crate) async fn handle_api_msg(&mut self, msg: RaftMsg<C, N, S>) -> Result<(), Fatal<C::NodeId>> {
        tracing::debug!("recv from rx_api: {}", msg.summary());

        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                self.handle_append_entries_request(rpc, tx).await?;
            }
            RaftMsg::RequestVote { rpc, tx } => {
                self.handle_vote_request(rpc, tx).await?;
            }
            RaftMsg::VoteResponse { target, resp, vote } => {
                if self.does_vote_match(&vote, "VoteResponse") {
                    self.handle_vote_resp(resp, target).await?;
                }
            }
            RaftMsg::InstallSnapshot { rpc, tx } => {
                self.handle_install_snapshot_request(rpc, tx).await?;
            }
            RaftMsg::BuildingSnapshotResult { result } => {
                self.handle_building_snapshot_result(result).await?;
            }
            RaftMsg::CheckIsLeaderRequest { tx } => {
                if self.engine.state.is_leader(&self.engine.config.id) {
                    self.handle_check_is_leader_request(tx).await?;
                } else {
                    self.reject_with_forward_to_leader(tx);
                }
            }
            RaftMsg::ClientWriteRequest { payload: rpc, tx } => {
                if self.engine.state.is_leader(&self.engine.config.id) {
                    self.write_entry(rpc, Some(tx)).await?;
                } else {
                    self.reject_with_forward_to_leader(tx);
                }
            }
            RaftMsg::Initialize { members, tx } => {
                self.handle_initialize(members, tx).await?;
            }
            RaftMsg::AddLearner { id, node, tx } => {
                if self.engine.state.is_leader(&self.engine.config.id) {
                    self.add_learner(id, node, tx).await?;
                } else {
                    self.reject_with_forward_to_leader(tx);
                }
            }
            RaftMsg::ChangeMembership {
                changes,
                when,
                turn_to_learner,
                tx,
            } => {
                if self.engine.state.is_leader(&self.engine.config.id) {
                    self.change_membership(changes, when, turn_to_learner, tx).await?;
                } else {
                    self.reject_with_forward_to_leader(tx);
                }
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
                            self.run_engine_commands::<Entry<C>>(&[]).await?;
                            tracing::debug!("ExternalCommand: triggered election");
                        } else {
                            // Node is switched to learner after setting up next election time.
                        }
                    }
                    ExternalCommand::Heartbeat => {
                        // TODO: reject if it is not leader?
                        let log_id = self.write_entry(EntryPayload::Blank, None).await?;
                        tracing::debug!(log_id = display(&log_id), "ExternalCommand: sent heartbeat log");
                    }
                    ExternalCommand::Snapshot => self.trigger_snapshot_if_needed(true).await,
                }
            }
            RaftMsg::Tick { i } => {
                // check every timer

                let now = Instant::now();
                tracing::debug!("received tick: {}, now: {:?}", i, now);

                let current_vote = self.engine.state.get_vote();

                // Follower/Candidate timer: next election
                if let Some(t) = self.next_election_time.get_time(current_vote) {
                    #[allow(clippy::collapsible_else_if)]
                    if now < t {
                        // timeout has not expired.
                    } else {
                        #[allow(clippy::collapsible_else_if)]
                        if self.runtime_config.enable_elect.load(Ordering::Relaxed) {
                            if self.engine.state.membership_state.effective().is_voter(&self.id) {
                                self.engine.elect();
                                self.run_engine_commands::<Entry<C>>(&[]).await?;
                            } else {
                                // Node is switched to learner after setting up next election time.
                            }
                        }
                    }
                }

                // TODO: test: with heartbeat log, election is automatically rejected.
                // TODO: test: fixture: make isolated_nodes a single-way isolating.

                // Leader send heartbeat
                let heartbeat_at = self.leader_data.as_ref().map(|x| x.next_heartbeat);
                if let Some(t) = heartbeat_at {
                    if now >= t {
                        if self.runtime_config.enable_heartbeat.load(Ordering::Relaxed) {
                            // heartbeat by sending a blank log
                            // TODO: use Engine::append_blank_log
                            let log_id = self.write_entry(EntryPayload::Blank, None).await?;
                            tracing::debug!(log_id = display(&log_id), "sent heartbeat log");
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
                self.run_engine_commands::<Entry<C>>(&[]).await?;
            }

            RaftMsg::HigherVote {
                target: _,
                higher,
                vote,
            } => {
                if self.does_vote_match(&vote, "HigherVote") {
                    // Rejected vote change is ok.
                    let _ = self.engine.vote_handler().handle_message_vote(&higher);
                    self.run_engine_commands::<Entry<C>>(&[]).await?;
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
                    self.handle_replication_progress(target, id, result).await?;
                }
            }
            RaftMsg::ReplicationFatal => {
                return Err(Fatal::Stopped);
            }
        };
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn handle_replication_progress(
        &mut self,
        target: C::NodeId,
        id: u64,
        result: Result<ReplicationResult<C::NodeId>, String>,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(
            target = display(target),
            result = debug(&result),
            "handle_replication_progress"
        );

        if tracing::enabled!(Level::DEBUG) {
            if let Some(l) = &self.leader_data {
                if !l.nodes.contains_key(&target) {
                    tracing::warn!("leader has removed target: {}", target);
                };
            } else {
                // TODO: A leader may have stepped down.
            }
        }

        // TODO: A leader may have stepped down.
        if self.engine.internal_server_state.is_leading() {
            self.engine.replication_handler().update_progress(target, id, result);
            self.run_engine_commands::<Entry<C>>(&[]).await?;
        }

        Ok(())
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
        if vote != self.engine.state.get_vote() {
            tracing::warn!(
                "vote changed: msg sent by: {:?}; curr: {}; ignore when ({})",
                vote,
                self.engine.state.get_vote(),
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

        if session_id.membership_log_id != self.engine.state.membership_state.effective().log_id {
            tracing::warn!(
                "membership_log_id changed: msg sent by: {}; curr: {}; ignore when ({})",
                session_id.membership_log_id.summary(),
                self.engine.state.membership_state.effective().log_id.summary(),
                msg
            );
            return false;
        }
        true
    }
}

#[async_trait::async_trait]
impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftRuntime<C> for RaftCore<C, N, S> {
    async fn run_command<'e, Ent>(
        &mut self,
        input_ref_entries: &'e [Ent],
        cur: &mut usize,
        cmd: Command<C::NodeId, C::Node>,
    ) -> Result<(), StorageError<C::NodeId>>
    where
        Ent: RaftLogId<C::NodeId> + Sync + Send + 'e,
        &'e Ent: Into<Entry<C>>,
    {
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
            Command::AppendInputEntries { range } => {
                let entry_refs = &input_ref_entries[range.clone()];

                let mut entries = Vec::with_capacity(entry_refs.len());
                for ent in entry_refs.iter() {
                    entries.push(ent.into())
                }

                // Build a slice of references.
                let entry_refs = entries.iter().collect::<Vec<_>>();

                self.storage.append_to_log(&entry_refs).await?
            }
            Command::AppendBlankLog { log_id } => {
                let ent = Entry {
                    log_id,
                    payload: EntryPayload::Blank,
                };
                let entry_refs = vec![&ent];
                self.storage.append_to_log(&entry_refs).await?
            }
            Command::MoveInputCursorBy { n } => *cur += n,
            Command::SaveVote { vote } => {
                self.storage.save_vote(&vote).await?;
            }
            Command::InstallElectionTimer { can_be_leader } => {
                self.set_next_election_time(can_be_leader);
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
                    for node in l.nodes.values() {
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
                    // TODO(2): consider remove the returned error from new_client().
                    // Node may not exist because `RaftNetworkFactory::new_client()` returns an
                    // error.
                    let node = &l.nodes.get(&target);

                    if let Some(node) = node {
                        match req {
                            Inflight::None => {
                                unreachable!("Inflight::None");
                            }
                            Inflight::Logs { id, log_id_range } => {
                                let _ = node.tx_repl.send(Replicate::Logs { id, log_id_range });
                            }
                            Inflight::Snapshot { id, last_log_id } => {
                                let snapshot = self.storage.get_current_snapshot().await?;
                                tracing::debug!("snapshot: {}", snapshot.as_ref().map(|x| &x.meta).summary());

                                if let Some(snapshot) = snapshot {
                                    debug_assert_eq!(last_log_id, snapshot.meta.last_log_id);
                                    let _ = node.tx_repl.send(Replicate::Snapshot { id, snapshot });
                                } else {
                                    unreachable!("No snapshot");
                                }
                            }
                        }
                    } else {
                        // TODO(2): if no such node, return an RemoteError?
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
                        l.nodes.insert(*target, handle);
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
