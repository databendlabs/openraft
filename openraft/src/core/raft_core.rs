use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Display;
use std::mem::swap;
use std::sync::Arc;

use futures::future::AbortHandle;
use futures::future::Abortable;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use futures::TryFutureExt;
use maplit::btreeset;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::sleep_until;
use tokio::time::timeout;
use tokio::time::Duration;
use tokio::time::Instant;
use tracing::trace_span;
use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::config::Config;
use crate::config::SnapshotPolicy;
use crate::core::replication::snapshot_is_within_half_of_threshold;
use crate::core::replication_lag;
use crate::core::Expectation;
use crate::core::InternalMessage;
use crate::core::ServerState;
use crate::core::SnapshotState;
use crate::core::SnapshotUpdate;
use crate::core::VoteWiseTime;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::EngineConfig;
use crate::entry::EntryRef;
use crate::error::AddLearnerError;
use crate::error::ChangeMembershipError;
use crate::error::CheckIsLeaderError;
use crate::error::ClientWriteError;
use crate::error::EmptyMembership;
use crate::error::ExtractFatal;
use crate::error::Fatal;
use crate::error::ForwardToLeader;
use crate::error::InProgress;
use crate::error::InitializeError;
use crate::error::LearnerIsLagging;
use crate::error::LearnerNotFound;
use crate::error::QuorumNotEnough;
use crate::error::RPCError;
use crate::error::Timeout;
use crate::error::VoteError;
use crate::metrics::RaftMetrics;
use crate::metrics::RemoveTarget;
use crate::metrics::ReplicationMetrics;
use crate::metrics::UpdateMatchedLogId;
use crate::progress::Progress;
use crate::quorum::QuorumSet;
use crate::raft::AddLearnerResponse;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::ClientWriteResponse;
use crate::raft::RaftMsg;
use crate::raft::RaftRespTx;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft_types::LogIdOptionExt;
use crate::raft_types::RaftLogId;
use crate::replication::ReplicationCore;
use crate::replication::ReplicationStream;
use crate::replication::UpdateReplication;
use crate::runtime::RaftRuntime;
use crate::storage::RaftSnapshotBuilder;
use crate::storage::Snapshot;
use crate::storage::StorageHelper;
use crate::versioned::Updatable;
use crate::versioned::Versioned;
use crate::ChangeMembers;
use crate::Entry;
use crate::EntryPayload;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::Node;
use crate::RPCTypes;
use crate::RaftNetwork;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Update;
use crate::Vote;

/// Data for a Leader.
///
/// It is created when RaftCore enters leader state, and will be dropped when it quits leader state.
pub(crate) struct LeaderData<C: RaftTypeConfig> {
    /// Channels to send result back to client when logs are committed.
    pub(crate) client_resp_channels: BTreeMap<u64, RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>>,

    /// A mapping of node IDs the replication state of the target node.
    // TODO(xp): make it a field of RaftCore. it does not have to belong to leader.
    //           It requires the Engine to emit correct add/remove replication commands
    pub(super) nodes: BTreeMap<C::NodeId, ReplicationStream<C::NodeId>>,

    /// The metrics of all replication streams
    pub(crate) replication_metrics: Versioned<ReplicationMetrics<C::NodeId>>,
}

impl<C: RaftTypeConfig> LeaderData<C> {
    pub(crate) fn new() -> Self {
        Self {
            client_resp_channels: Default::default(),
            nodes: BTreeMap::new(),
            replication_metrics: Versioned::new(ReplicationMetrics::default()),
        }
    }
}

/// The core type implementing the Raft protocol.
pub struct RaftCore<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    /// This node's ID.
    pub(crate) id: C::NodeId,

    /// This node's runtime config.
    pub(crate) config: Arc<Config>,

    /// The `RaftNetworkFactory` implementation.
    pub(crate) network: N,

    /// The `RaftStorage` implementation.
    pub(crate) storage: S,

    pub(crate) engine: Engine<C::NodeId>,

    pub(crate) leader_data: Option<LeaderData<C>>,

    /// The node's current snapshot state.
    pub(crate) snapshot_state: Option<SnapshotState<S::SnapshotData>>,

    /// The last time a heartbeat was received.
    pub(crate) last_heartbeat: Option<Instant>,

    /// The time to elect if a follower does not receive any append-entry message.
    pub(crate) next_election_time: VoteWiseTime<C::NodeId>,

    tx_internal: mpsc::Sender<InternalMessage<C::NodeId>>,

    pub(crate) rx_internal: mpsc::Receiver<InternalMessage<C::NodeId>>,

    pub(crate) tx_api: mpsc::UnboundedSender<RaftMsg<C, N, S>>,
    pub(crate) rx_api: mpsc::UnboundedReceiver<RaftMsg<C, N, S>>,

    tx_metrics: watch::Sender<RaftMetrics<C::NodeId>>,

    pub(crate) rx_shutdown: oneshot::Receiver<()>,

    pub(crate) span: Span,
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    pub(crate) fn spawn(
        id: C::NodeId,
        config: Arc<Config>,
        network: N,
        storage: S,
        tx_api: mpsc::UnboundedSender<RaftMsg<C, N, S>>,
        rx_api: mpsc::UnboundedReceiver<RaftMsg<C, N, S>>,
        tx_metrics: watch::Sender<RaftMetrics<C::NodeId>>,
        rx_shutdown: oneshot::Receiver<()>,
    ) -> JoinHandle<Result<(), Fatal<C::NodeId>>> {
        let (tx_internal, rx_internal) = mpsc::channel(1024);

        let span = tracing::span!(
            parent: tracing::Span::current(),
            Level::DEBUG,
            "RaftCore",
            id = display(id),
            cluster = display(&config.cluster_name)
        );

        let this = Self {
            id,
            config,
            network,
            storage,

            engine: Engine::default(),
            leader_data: None,

            snapshot_state: None,
            last_heartbeat: None,
            next_election_time: VoteWiseTime::new(Vote::default(), Instant::now() + Duration::from_secs(86400)),

            tx_internal,
            rx_internal,

            tx_api,
            rx_api,

            tx_metrics,

            rx_shutdown,

            span,
        };

        tokio::spawn(this.main().instrument(trace_span!("spawn").or_current()))
    }

    /// The main loop of the Raft protocol.
    async fn main(mut self) -> Result<(), Fatal<C::NodeId>> {
        let span = tracing::span!(parent: &self.span, Level::DEBUG, "main");
        let res = self.do_main().instrument(span).await;
        match res {
            Ok(_) => Ok(()),
            Err(err) => {
                tracing::error!(?err, "quit RaftCore::main on error");

                let mut curr = self.tx_metrics.borrow().clone();
                curr.running_state = Err(err.clone());
                let _ = self.tx_metrics.send(curr);

                Err(err)
            }
        }
    }

    #[tracing::instrument(level="trace", skip(self), fields(id=display(self.id), cluster=%self.config.cluster_name))]
    async fn do_main(&mut self) -> Result<(), Fatal<C::NodeId>> {
        tracing::debug!("raft node is initializing");

        let state = {
            let mut helper = StorageHelper::new(&mut self.storage);
            helper.get_initial_state().await?
        };

        // TODO(xp): this is not necessary.
        self.storage.save_vote(&state.vote).await?;

        self.engine = Engine::new(self.id, &state, EngineConfig {
            max_applied_log_to_keep: self.config.max_applied_log_to_keep,
            purge_batch_size: self.config.purge_batch_size,
            keep_unsnapshoted_log: self.config.keep_unsnapshoted_log,
        });

        self.engine.state.last_applied = state.last_applied;

        // NOTE: The commit index must be determined by a leader after
        // successfully committing a new log to the cluster.
        self.engine.state.committed = None;

        // Fetch the most recent snapshot in the system.
        if let Some(snapshot) = self.storage.get_current_snapshot().await? {
            self.engine.snapshot_last_log_id = Some(snapshot.meta.last_log_id);
            self.engine.metrics_flags.set_data_changed();
        }

        let has_log = self.engine.state.last_log_id().is_some();
        let single = self.engine.state.membership_state.effective.voter_ids().count() <= 1;
        let is_voter = self.engine.state.membership_state.effective.membership.is_voter(&self.id);

        const HAS_LOG: bool = true;
        const NO_LOG: bool = false;

        const SINGLE: bool = true;
        const MULTI: bool = false;

        const IS_VOTER: bool = true;
        const IS_LEARNER: bool = false;

        self.engine.state.server_state = match (has_log, single, is_voter) {
            // A restarted raft that already received some logs but was not yet added to a cluster.
            // It should remain in Learner state, not Follower.
            (HAS_LOG, SINGLE, IS_LEARNER) => ServerState::Learner,
            (HAS_LOG, MULTI, IS_LEARNER) => ServerState::Learner,

            (NO_LOG, SINGLE, IS_LEARNER) => ServerState::Learner, // impossible: no logs but there are other members.
            (NO_LOG, MULTI, IS_LEARNER) => ServerState::Learner,  // impossible: no logs but there are other members.

            // If this is the only configured member and there is live state, then this is
            // a single-node cluster. It should become the leader at once.
            (HAS_LOG, SINGLE, IS_VOTER) => ServerState::Candidate,

            // The initial state when a raft is created from empty store.
            (NO_LOG, SINGLE, IS_VOTER) => ServerState::Learner,

            // Otherwise it is Follower.
            (HAS_LOG, MULTI, IS_VOTER) => ServerState::Follower,

            (NO_LOG, MULTI, IS_VOTER) => ServerState::Follower, // impossible: no logs but there are other members.
        };

        if self.engine.state.server_state == ServerState::Follower {
            // To ensure that restarted nodes don't disrupt a stable cluster.
            self.set_next_election_time(false);
        }

        tracing::debug!("id={} target_state: {:?}", self.id, self.engine.state.server_state);

        // This is central loop of the system. The Raft core assumes a few different roles based
        // on cluster state. The Raft core will delegate control to the different state
        // controllers and simply awaits the delegated loop to return, which will only take place
        // if some error has been encountered, or if a state change is required.
        loop {
            self.leader_data = None;

            match &self.engine.state.server_state {
                ServerState::Leader => {
                    self.leader_data = Some(LeaderData::new());
                    self.leader_loop().await?;
                }
                ServerState::Candidate => self.candidate_loop().await?,
                ServerState::Follower => self.follower_learner_loop(ServerState::Follower).await?,
                ServerState::Learner => self.follower_learner_loop(ServerState::Learner).await?,
                ServerState::Shutdown => {
                    tracing::info!("node has shutdown");
                    return Ok(());
                }
            }
        }
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
    pub(super) async fn handle_check_is_leader_request(&mut self, tx: RaftRespTx<(), CheckIsLeaderError<C::NodeId>>) {
        // Setup sentinel values to track when we've received majority confirmation of leadership.

        let em = &self.engine.state.membership_state.effective;
        let mut granted = btreeset! {self.id};

        if em.is_quorum(granted.iter()) {
            let _ = tx.send(Ok(()));
            return;
        }

        // Spawn parallel requests, all with the standard timeout for heartbeats.
        let mut pending = FuturesUnordered::new();

        let voter_progresses = if let Some(l) = &self.engine.state.leader {
            l.progress
                .iter()
                .filter(|(id, _v)| l.progress.is_voter(id) == Some(true))
                .copied()
                .collect::<Vec<_>>()
        } else {
            unreachable!("it has to be a leader!!!");
        };

        for (target, matched) in voter_progresses {
            if target == self.id {
                continue;
            }

            let rpc = AppendEntriesRequest {
                vote: self.engine.state.vote,
                prev_log_id: matched,
                entries: vec![],
                leader_commit: self.engine.state.committed,
            };

            let my_id = self.id;
            let target_node = self.engine.state.membership_state.effective.get_node(&target).cloned();
            let mut network = self.network.connect(target, target_node.as_ref()).await;

            let ttl = Duration::from_millis(self.config.heartbeat_interval);

            let task = tokio::spawn(
                async move {
                    let outer_res = timeout(ttl, network.send_append_entries(rpc)).await;
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

            // If we receive a response with a greater term, then revert to follower and abort this request.
            if let AppendEntriesResponse::HigherVote(vote) = data {
                // TODO: there is no guarantee the response vote is greater than local. Because local vote may already
                //       changed.
                assert!(vote > self.engine.state.vote);
                self.engine.state.vote = vote;
                // TODO(xp): deal with storage error
                self.save_vote().await.unwrap();
                // TODO(xp): if receives error about a higher term, it should stop at once?
                self.set_target_state(ServerState::Follower);
            }

            granted.insert(target);

            let mem = &self.engine.state.membership_state.effective;
            if mem.is_quorum(granted.iter()) {
                let _ = tx.send(Ok(()));
                return;
            }
        }

        // If we've hit this location, then we've failed to gather needed confirmations due to
        // request failures.

        let _ = tx.send(Err(QuorumNotEnough {
            cluster: self.engine.state.membership_state.effective.membership.summary(),
            got: granted,
        }
        .into()));
    }

    /// Add a new node to the cluster as a learner, bringing it up-to-speed, and then responding
    /// on the given channel.
    ///
    /// Adding a learner does not affect election, thus it does not need to enter joint consensus.
    ///
    /// TODO: It has to wait for the previous membership to commit.
    /// TODO: Otherwise a second proposed membership implies the previous one is committed.
    /// TODO: Test it.
    /// TODO: This limit can be removed if membership_state is replaced by a list of membership logs.
    /// TODO: Because allowing this requires the engine to be able to store more than 2 membership logs.
    /// And it does not need to wait for the previous membership log to commit to propose the new membership log.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) async fn add_learner(
        &mut self,
        target: C::NodeId,
        node: Option<Node>,
        tx: RaftRespTx<AddLearnerResponse<C::NodeId>, AddLearnerError<C::NodeId>>,
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

        // Ensure the node doesn't already exist in the current
        // config, in the set of new nodes already being synced, or in the nodes being removed.

        let curr = &self.engine.state.membership_state.effective;
        if curr.contains(&target) {
            let matched = if let Some(l) = &self.engine.state.leader {
                *l.progress.get(&target)
            } else {
                unreachable!("it has to be a leader!!!");
            };

            tracing::debug!(
                "target {:?} already member or learner, can't add; matched:{:?}",
                target,
                matched
            );

            let _ = tx.send(Ok(AddLearnerResponse {
                membership_log_id: self.engine.state.membership_state.effective.log_id,
                matched,
            }));
            return Ok(());
        }

        let curr = &self.engine.state.membership_state.effective.membership;
        let res = curr.add_learner(target, node);
        let new_membership = match res {
            Ok(x) => x,
            Err(e) => {
                let _ = tx.send(Err(AddLearnerError::MissingNodeInfo(e)));
                return Ok(());
            }
        };

        tracing::debug!(?new_membership, "new_membership with added learner: {}", target);

        let log_id = self.write_entry(EntryPayload::Membership(new_membership), None).await?;

        tracing::debug!(
            "after add target node {} as learner {:?}",
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
    /// If `turn_to_learner` is `true`, removed `voter` will becomes `learner`. Otherwise they will be just removed.
    #[tracing::instrument(level = "debug", skip(self, tx))]
    pub(super) async fn change_membership(
        &mut self,
        changes: ChangeMembers<C::NodeId>,
        expectation: Option<Expectation>,
        turn_to_learner: bool,
        tx: RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>,
    ) -> Result<(), Fatal<C::NodeId>> {
        let last = self.engine.state.membership_state.effective.membership.get_joint_config().last().unwrap();
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

        let mem = &self.engine.state.membership_state.effective;
        let curr = mem.membership.clone();

        let old_members = mem.voter_ids().collect::<BTreeSet<_>>();
        let only_in_new = members.difference(&old_members);

        let new_config = {
            let res = curr.next_safe(members.clone(), turn_to_learner);
            match res {
                Ok(x) => x,
                Err(e) => {
                    let change_err = ChangeMembershipError::MissingNodeInfo(e);
                    let _ = tx.send(Err(ClientWriteError::ChangeMembershipError(change_err)));
                    return Ok(());
                }
            }
        };

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

    /// Check if the effective membership is committed, so that a new membership is allowed to be proposed.
    fn check_membership_committed(&self) -> Result<(), ChangeMembershipError<C::NodeId>> {
        let st = &self.engine.state;

        if st.is_membership_committed() {
            return Ok(());
        }

        Err(ChangeMembershipError::InProgress(InProgress {
            committed: st.committed,
            membership_log_id: st.membership_state.effective.log_id,
        }))
    }

    /// return Ok if all the current replication states satisfy the `expectation` for changing membership.
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

                    let matched = if let Some(l) = &self.engine.state.leader {
                        *l.progress.get(node_id)
                    } else {
                        unreachable!("it has to be a leader!!!");
                    };

                    let distance = replication_lag(&matched, &last_log_id);

                    if distance <= self.config.replication_lag_threshold {
                        continue;
                    }

                    let lagging = LearnerIsLagging {
                        node_id: *node_id,
                        matched,
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
    /// I.e.: append the log entry to local store, forward it to a quorum(including the leader), waiting for it to be
    /// committed and applied.
    ///
    /// The result of applying it to state machine is sent to `resp_tx`, if it is not `None`.
    /// The calling side may not receive a result from `resp_tx`, if raft is shut down.
    #[tracing::instrument(level = "debug", skip_all, fields(id = display(self.id)))]
    pub async fn write_entry(
        &mut self,
        payload: EntryPayload<C>,
        resp_tx: Option<RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>>,
    ) -> Result<LogId<C::NodeId>, Fatal<C::NodeId>> {
        tracing::debug!(payload = display(payload.summary()), "write_entry");

        let mut entry_refs = [EntryRef::new(&payload)];
        // TODO: it should returns membership config error etc. currently this is done by the caller.
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
    #[tracing::instrument(level = "trace", skip_all)]
    pub fn flush_metrics(&mut self) {
        if !self.engine.metrics_flags.changed() {
            return;
        }

        let leader_metrics = if self.engine.metrics_flags.leader {
            let replication_metrics = self.leader_data.as_ref().map(|x| x.replication_metrics.clone());
            Update::Update(replication_metrics)
        } else {
            Update::AsIs
        };

        self.report_metrics(leader_metrics);
        self.engine.metrics_flags.reset();
    }

    /// Report a metrics payload on the current state of the Raft node.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn report_metrics(&self, replication: Update<Option<Versioned<ReplicationMetrics<C::NodeId>>>>) {
        let replication = match replication {
            Update::Update(v) => v,
            Update::AsIs => self.tx_metrics.borrow().replication.clone(),
        };

        let m = RaftMetrics {
            running_state: Ok(()),
            id: self.id,

            // --- data ---
            current_term: self.engine.state.vote.term,
            last_log_index: self.engine.state.last_log_id().map(|id| id.index),
            last_applied: self.engine.state.last_applied,
            snapshot: self.engine.snapshot_last_log_id,

            // --- cluster ---
            state: self.engine.state.server_state,
            current_leader: self.current_leader(),
            membership_config: self.engine.state.membership_state.effective.clone(),

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
        member_nodes: BTreeMap<C::NodeId, Option<Node>>,
    ) -> Result<(), InitializeError<C::NodeId>> {
        let membership = Membership::try_from(member_nodes)?;
        let payload = EntryPayload::<C>::Membership(membership);

        let mut entry_refs = [EntryRef::new(&payload)];
        self.engine.initialize(&mut entry_refs)?;
        self.run_engine_commands(&entry_refs).await?;

        Ok(())
    }

    /// Save the Raft node's current hard state to disk.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) async fn save_vote(&mut self) -> Result<(), StorageError<C::NodeId>> {
        self.storage.save_vote(&self.engine.state.vote).await
    }

    /// Update core's target state, ensuring all invariants are upheld.
    #[tracing::instrument(level = "trace", skip(self), fields(id=display(self.id)))]
    pub(crate) fn set_target_state(&mut self, target_state: ServerState) {
        tracing::debug!(id = display(self.id), ?target_state, "set_target_state");

        if target_state == ServerState::Follower
            && !self.engine.state.membership_state.effective.membership.is_voter(&self.id)
        {
            self.engine.state.server_state = ServerState::Learner;
        } else {
            self.engine.state.server_state = target_state;
        }
    }

    /// Get the next election timeout, generating a new value if not set.
    /// TODO: get() should not have a side effect of updating the timer.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn get_next_election_time(&mut self) -> Instant {
        let current_vote = &self.engine.state.vote;

        let time = self.next_election_time.get_time(current_vote);
        if let Some(t) = time {
            t
        } else {
            let t = Duration::from_millis(self.config.new_rand_election_timeout());
            tracing::debug!("create election timeout after: {:?}", t);

            let t = Instant::now() + t;

            self.next_election_time = VoteWiseTime::new(*current_vote, t);

            t
        }
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

        self.next_election_time = VoteWiseTime::new(self.engine.state.vote, now + t);
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn reject_election_for_a_while(&mut self) {
        let now = Instant::now();
        self.last_heartbeat = Some(now);
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) async fn handle_internal_msg(
        &mut self,
        msg: InternalMessage<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        match msg {
            InternalMessage::SnapshotUpdate(update) => {
                self.update_snapshot_state(update);
            }
        }
        Ok(())
    }

    /// Update the system's snapshot state based on the given data.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn update_snapshot_state(&mut self, update: SnapshotUpdate<C::NodeId>) {
        if let SnapshotUpdate::SnapshotComplete(log_id) = update {
            self.engine.snapshot_last_log_id = Some(log_id);
            self.engine.metrics_flags.set_data_changed();
        }
        // If snapshot state is anything other than streaming, then drop it.
        if let Some(state @ SnapshotState::Streaming { .. }) = self.snapshot_state.take() {
            self.snapshot_state = Some(state);
        }
    }

    /// Trigger a log compaction (snapshot) job if needed.
    /// If force is True, it will skip the threshold check and start creating snapshot as demanded.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) async fn trigger_log_compaction_if_needed(&mut self, force: bool) {
        if self.snapshot_state.is_some() {
            return;
        }
        let SnapshotPolicy::LogsSinceLast(threshold) = &self.config.snapshot_policy;

        let last_applied = match self.engine.state.last_applied {
            None => {
                return;
            }
            Some(x) => x,
        };

        // Check to ensure we have actual entries for compaction.
        if Some(last_applied.index) < self.engine.snapshot_last_log_id.index() {
            return;
        }

        if !force {
            // If we are below the threshold, then there is nothing to do.
            if self.engine.state.last_applied.next_index() - self.engine.snapshot_last_log_id.next_index() < *threshold
            {
                return;
            }
        }

        // At this point, we are clear to begin a new compaction process.
        let mut builder = self.storage.get_snapshot_builder().await;
        let (handle, reg) = AbortHandle::new_pair();
        let (chan_tx, _) = broadcast::channel(1);
        let tx_internal = self.tx_internal.clone();
        self.snapshot_state = Some(SnapshotState::Snapshotting {
            handle,
            sender: chan_tx.clone(),
        });

        tokio::spawn(
            async move {
                let f = builder.build_snapshot();
                let res = Abortable::new(f, reg).await;
                match res {
                    Ok(res) => match res {
                        Ok(snapshot) => {
                            let _ = tx_internal.try_send(InternalMessage::SnapshotUpdate(
                                SnapshotUpdate::SnapshotComplete(snapshot.meta.last_log_id),
                            ));
                            // This will always succeed.
                            let _ = chan_tx.send(snapshot.meta.last_log_id.index);
                        }
                        Err(err) => {
                            tracing::error!({error=%err}, "error while generating snapshot");
                            let _ =
                                tx_internal.try_send(InternalMessage::SnapshotUpdate(SnapshotUpdate::SnapshotFailed));
                        }
                    },
                    Err(_aborted) => {
                        let _ = tx_internal.try_send(InternalMessage::SnapshotUpdate(SnapshotUpdate::SnapshotFailed));
                    }
                }
            }
            .instrument(tracing::debug_span!("beginning new log compaction process")),
        );
    }

    /// Reject a request due to the Raft node being in a state which prohibits the request.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    pub(crate) fn reject_with_forward_to_leader<T, E>(&self, tx: RaftRespTx<T, E>)
    where E: From<ForwardToLeader<C::NodeId>> {
        let l = self.current_leader();
        let err = ForwardToLeader {
            leader_id: l,
            leader_node: self.get_leader_node(l),
        };

        let _ = tx.send(Err(err.into()));
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub fn current_leader(&self) -> Option<C::NodeId> {
        if !self.engine.state.vote.committed {
            return None;
        }

        let id = self.engine.state.vote.node_id;

        if id == self.id {
            if self.engine.state.server_state == ServerState::Leader {
                Some(id)
            } else {
                None
            }
        } else {
            Some(id)
        }
    }

    pub(crate) fn get_leader_node(&self, leader_id: Option<C::NodeId>) -> Option<Node> {
        match leader_id {
            None => None,
            Some(id) => self.engine.state.membership_state.effective.get_node(&id).cloned(),
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn apply_to_state_machine(&mut self, upto_index: u64) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(upto_index = display(upto_index), "apply_to_state_machine");

        let since = self.engine.state.last_applied.next_index();
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
        self.engine.state.last_applied = Some(last_applied);

        tracing::debug!(last_applied = display(last_applied), "update last_applied");

        if let Some(l) = &mut self.leader_data {
            let mut results = apply_results.into_iter();

            for log_index in since..end {
                let tx = l.client_resp_channels.remove(&log_index);

                let i = log_index - since;
                let entry = &entries[i as usize];
                let apply_res = results.next().unwrap();

                Self::send_response(entry, apply_res, tx).await;
            }
        }

        self.trigger_log_compaction_if_needed(false).await;
        Ok(())
    }

    /// Send result of applying a log entry to its client.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) async fn send_response(
        entry: &Entry<C>,
        resp: C::R,
        tx: Option<RaftRespTx<ClientWriteResponse<C>, ClientWriteError<C::NodeId>>>,
    ) {
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

    /// Handle the post-commit logic for a client request.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(super) async fn leader_commit(&mut self, log_index: u64) -> Result<(), StorageError<C::NodeId>> {
        self.leader_step_down();

        self.apply_to_state_machine(log_index).await?;

        Ok(())
    }

    /// Begin replicating upto the given log id.
    ///
    /// It does not block until the entry is committed or actually sent out.
    /// It merely broadcasts a signal to inform the replication threads.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn replicate_entry(&mut self, log_id: LogId<C::NodeId>) {
        if let Some(l) = &self.leader_data {
            if tracing::enabled!(Level::DEBUG) {
                for node_id in l.nodes.keys() {
                    tracing::debug!(node_id = display(node_id), log_id = display(log_id), "replicate_entry");
                }
            }

            for node in l.nodes.values() {
                let _ = node.repl_tx.send(UpdateReplication {
                    last_log_id: Some(log_id),
                    committed: self.engine.state.committed,
                });
            }
        } else {
            unreachable!("it has to be a leader!!!");
        }
    }

    /// Spawn a new replication stream returning its replication state handle.
    #[tracing::instrument(level = "debug", skip(self))]
    #[allow(clippy::type_complexity)]
    pub(crate) async fn spawn_replication_stream(&mut self, target: C::NodeId) -> ReplicationStream<C::NodeId> {
        let target_node = self.engine.state.membership_state.effective.get_node(&target);

        ReplicationCore::<C, N, S>::spawn(
            target,
            target_node.cloned(),
            self.engine.state.vote,
            self.config.clone(),
            self.engine.state.last_log_id(),
            self.engine.state.committed,
            self.network.connect(target, target_node).await,
            self.storage.get_log_reader().await,
            self.tx_api.clone(),
            tracing::span!(parent: &self.span, Level::DEBUG, "replication", id=display(self.id), target=display(target)),
        )
    }

    /// Remove a replication if the membership that does not include it has committed.
    ///
    /// Return true if removed.
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn remove_replication(&mut self, target: C::NodeId) -> bool {
        tracing::info!("removed_replication to: {}", target);

        let repl_state = if let Some(l) = &mut self.leader_data {
            l.nodes.remove(&target)
        } else {
            unreachable!("it has to be a leader!!!");
        };

        if let Some(s) = repl_state {
            let handle = s.handle;

            // Drop sender to notify the task to shutdown
            drop(s.repl_tx);

            tracing::debug!("joining removed replication: {}", target);
            let _x = handle.await;
            tracing::info!("Done joining removed replication : {}", target);
        } else {
            unreachable!("try to nonexistent replication to {}", target);
        }

        if let Some(l) = &mut self.leader_data {
            l.replication_metrics.update(RemoveTarget { target });
        } else {
            unreachable!("It has to be a leader!!!");
        }

        // TODO(xp): set_replication_metrics_changed() can be removed.
        //           Use self.replication_metrics.version to detect changes.
        self.engine.metrics_flags.set_replication_changed();

        true
    }

    /// Leader will keep working until the effective membership that removes it committed.
    ///
    /// This is ony called by leader.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn leader_step_down(&mut self) {
        let em = &self.engine.state.membership_state.effective;

        if self.engine.state.committed < em.log_id {
            return;
        }

        // TODO: Leader does not need to step down. It can keep working.
        //       This requires to separate Leader(Proposer) and Acceptors.
        if !em.is_voter(&self.id) {
            tracing::debug!("leader is stepping down");

            // TODO(xp): transfer leadership
            self.set_target_state(ServerState::Learner);
            self.engine.metrics_flags.set_cluster_changed();
        }
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
            tracing::debug!("run command: start...");
            for c in self.engine.commands.iter() {
                tracing::debug!("run command: {:?}", c);
            }
        }

        let mut curr = 0;
        let mut commands = vec![];
        swap(&mut self.engine.commands, &mut commands);
        for cmd in commands {
            self.run_command(input_entries, &mut curr, &cmd).await?;
        }

        Ok(())
    }

    #[tracing::instrument(level="debug", skip_all, fields(id=display(self.id), raft_state="leader"))]
    pub(crate) async fn leader_loop(&mut self) -> Result<(), Fatal<C::NodeId>> {
        // Setup state as leader.
        self.last_heartbeat = None;
        debug_assert!(self.engine.state.vote.committed);

        // Spawn replication streams for followers and learners.

        let targets = {
            let mem = &self.engine.state.membership_state.effective;

            let node_ids = mem.nodes().map(|(&nid, _)| nid);
            node_ids.filter(|elem| elem != &self.id).collect::<Vec<_>>()
        };

        // TODO(xp): make this Engine::Command driven.
        for target in targets {
            let state = self.spawn_replication_stream(target).await;
            if let Some(l) = &mut self.leader_data {
                l.nodes.insert(target, state);
            } else {
                unreachable!("it has to be a leader!!!");
            }
        }

        // Commit the initial entry when new leader established.
        self.write_entry(EntryPayload::Blank, None).await?;

        // report the leader metrics every time there came to a new leader
        // if not `report_metrics` before the leader loop, the leader metrics may not be updated cause no coming event.

        let replication_metrics = if let Some(l) = &self.leader_data {
            l.replication_metrics.clone()
        } else {
            unreachable!("it has to be a leader!!!");
        };
        self.report_metrics(Update::Update(Some(replication_metrics)));

        loop {
            if !self.engine.state.server_state.is_leader() {
                tracing::info!("id={} state becomes: {:?}", self.id, self.engine.state.server_state);

                // implicit drop replication_rx
                // notify to all nodes DO NOT send replication event any more.
                return Ok(());
            }

            self.flush_metrics();

            tokio::select! {
                Some(msg) = self.rx_api.recv() => {
                    self.handle_api_msg(msg).await?;
                },

                Some(internal_msg) = self.rx_internal.recv() => {
                    tracing::info!("leader recv from rx_internal: {:?}", internal_msg);
                    self.handle_internal_msg(internal_msg).await?;
                }

                Ok(_) = &mut self.rx_shutdown => {
                    tracing::info!("leader recv from rx_shudown");
                    self.set_target_state(ServerState::Shutdown);
                }
            }
        }
    }

    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.id), raft_state="candidate"))]
    async fn candidate_loop(&mut self) -> Result<(), Fatal<C::NodeId>> {
        // report the new state before enter the loop
        self.report_metrics(Update::Update(None));

        loop {
            if !self.engine.state.server_state.is_candidate() {
                return Ok(());
            }

            self.flush_metrics();

            self.set_next_election_time(true);

            self.engine.elect();
            self.run_engine_commands::<Entry<C>>(&[]).await?;

            loop {
                if !self.engine.state.server_state.is_candidate() {
                    return Ok(());
                }

                let timeout_fut = sleep_until(self.get_next_election_time());

                tokio::select! {
                    _ = timeout_fut => {
                        // This election has timed-out. Leave to the caller to decide what to do.
                        break;
                    },

                    Some(msg) = self.rx_api.recv() => {
                        self.handle_api_msg(msg).await?;
                    },

                    Some(internal_msg) = self.rx_internal.recv() => {
                        self.handle_internal_msg(internal_msg).await?;
                    },

                    Ok(_) = &mut self.rx_shutdown => self.set_target_state(ServerState::Shutdown),
                }
            }
        }
    }

    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.id)))]
    async fn follower_learner_loop(&mut self, server_state: ServerState) -> Result<(), Fatal<C::NodeId>> {
        // report the new state before enter the loop
        self.report_metrics(Update::Update(None));

        loop {
            if self.engine.state.server_state != server_state {
                return Ok(());
            }

            self.flush_metrics();

            tokio::select! {
                Some(msg) = self.rx_api.recv() => {
                    self.handle_api_msg(msg).await?;
                },

                Some(internal_msg) = self.rx_internal.recv() => self.handle_internal_msg(internal_msg).await?,

                Ok(_) = &mut self.rx_shutdown => self.set_target_state(ServerState::Shutdown),
            }
        }
    }

    /// Spawn parallel vote requests to all cluster members.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn spawn_parallel_vote_requests(&mut self, vote_req: &VoteRequest<C::NodeId>) {
        let members = self.engine.state.membership_state.effective.voter_ids();

        let vote = vote_req.vote;

        for target in members {
            if target == self.id {
                continue;
            }

            let req = vote_req.clone();
            let target_node = self.engine.state.membership_state.effective.get_node(&target).cloned();
            let mut network = self.network.connect(target, target_node.as_ref()).await;
            let tx = self.tx_api.clone();

            let _ = tokio::spawn(
                async move {
                    let res = network.send_vote(req).await;

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
    ) -> Result<VoteResponse<C::NodeId>, VoteError<C::NodeId>> {
        tracing::debug!(req = display(req.summary()), "handle_vote_request");

        // TODO(xp): Checking last_heartbeat can be removed,
        //           if we have finished using blank log for heartbeat:
        //           https://github.com/datafuselabs/openraft/issues/151
        // Do not respond to the request if we've received a heartbeat within the election timeout minimum.
        if let Some(inst) = &self.last_heartbeat {
            let now = Instant::now();
            let delta = now.duration_since(*inst);
            if self.config.election_timeout_min >= (delta.as_millis() as u64) {
                tracing::debug!(
                    %req.vote,
                    ?delta,
                    "rejecting vote request received within election timeout minimum"
                );
                return Ok(VoteResponse {
                    vote: self.engine.state.vote,
                    vote_granted: false,
                    last_log_id: self.engine.state.last_log_id(),
                });
            }
        }

        let resp = self.engine.handle_vote_req(req);
        self.run_engine_commands::<Entry<C>>(&[]).await?;

        Ok(resp)
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
            my_vote = display(&self.engine.state.vote),
            my_last_log_id = debug(self.engine.state.last_log_id()),
            "recv vote response"
        );

        self.engine.handle_vote_resp(target, resp);
        self.run_engine_commands::<Entry<C>>(&[]).await?;

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = debug(self.engine.state.server_state), id=display(self.id)))]
    pub(crate) async fn handle_api_msg(&mut self, msg: RaftMsg<C, N, S>) -> Result<(), Fatal<C::NodeId>> {
        tracing::debug!("recv from rx_api: {}", msg.summary());

        let is_leader = || self.engine.state.server_state == ServerState::Leader;

        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                let resp =
                    self.engine.handle_append_entries_req(&rpc.vote, rpc.prev_log_id, &rpc.entries, rpc.leader_commit);
                self.run_engine_commands(rpc.entries.as_slice()).await?;
                let _ = tx.send(Ok(resp));
            }
            RaftMsg::RequestVote { rpc, tx } => {
                let _ = tx.send(self.handle_vote_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::VoteResponse { target, resp, vote } => {
                if self.does_server_state_count_match(vote, "RevertToFollower") {
                    self.handle_vote_resp(resp, target).await?;
                }
            }
            RaftMsg::InstallSnapshot { rpc, tx } => {
                let _ = tx.send(self.handle_install_snapshot_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::CheckIsLeaderRequest { tx } => {
                if is_leader() {
                    self.handle_check_is_leader_request(tx).await;
                } else {
                    self.reject_with_forward_to_leader(tx);
                }
            }
            RaftMsg::ClientWriteRequest { rpc, tx } => {
                if is_leader() {
                    self.write_entry(rpc.payload, Some(tx)).await?;
                } else {
                    self.reject_with_forward_to_leader(tx);
                }
            }
            RaftMsg::Initialize { members, tx } => {
                let _ = tx.send(self.handle_initialize(members).await.extract_fatal()?);
            }
            RaftMsg::AddLearner { id, node, tx } => {
                if is_leader() {
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
                if is_leader() {
                    self.change_membership(changes, when, turn_to_learner, tx).await?;
                } else {
                    self.reject_with_forward_to_leader(tx);
                }
            }
            RaftMsg::ExternalRequest { req } => {
                req(&self.engine.state, &mut self.storage, &mut self.network);
            }
            RaftMsg::Tick { i } => {
                // check every timer

                tracing::debug!("received tick: {}", i);

                let current_vote = &self.engine.state.vote;

                // Follower timer: next election
                if let Some(t) = self.next_election_time.get_time(current_vote) {
                    #[allow(clippy::collapsible_else_if)]
                    if Instant::now() < t {
                        // timeout has not expired.
                    } else {
                        if self.engine.state.membership_state.effective.is_voter(&self.id) {
                            self.set_target_state(ServerState::Candidate)
                        } else {
                            // Node is switched to learner after setting up next election time.
                        }
                    }
                }
            }

            RaftMsg::RevertToFollower { target, new_vote, vote } => {
                if self.does_server_state_count_match(vote, "RevertToFollower") {
                    self.handle_revert_to_follower(target, new_vote).await?;
                }
            }

            RaftMsg::UpdateReplicationMatched { target, result, vote } => {
                if self.does_server_state_count_match(vote, "UpdateReplicationMatched") {
                    self.handle_update_matched(target, result).await?;
                }
            }

            RaftMsg::NeedsSnapshot {
                target: _,
                must_include,
                tx,
                vote,
            } => {
                if self.does_server_state_count_match(vote, "NeedsSnapshot") {
                    self.handle_needs_snapshot(must_include, tx).await?;
                }
            }
            RaftMsg::ReplicationFatal => {
                self.set_target_state(ServerState::Shutdown);
            }
        };
        Ok(())
    }

    /// Handle events from replication streams for when this node needs to revert to follower state.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn handle_revert_to_follower(
        &mut self,
        _target: C::NodeId,
        vote: Vote<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        if vote > self.engine.state.vote {
            self.engine.state.vote = vote;
            self.save_vote().await?;
            // TODO: when switching to Follower, the next election time has to be set.
            self.set_target_state(ServerState::Follower);
        }
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn handle_update_matched(
        &mut self,
        target: C::NodeId,
        result: Result<LogId<C::NodeId>, String>,
    ) -> Result<(), StorageError<C::NodeId>> {
        // Update target's match index & check if it is awaiting removal.

        tracing::debug!(
            target = display(target),
            result = debug(&result),
            "handle_update_matched"
        );

        // TODO(xp): a leader has to refuse a message from a previous leader.
        if let Some(l) = &self.leader_data {
            if !l.nodes.contains_key(&target) {
                return Ok(());
            };
        } else {
            // no longer a leader.
            tracing::warn!(
                target = display(target),
                result = debug(&result),
                "received replication update but no longer a leader"
            );
            return Ok(());
        }

        tracing::debug!("update matched: {:?}", result);

        let matched = match result {
            Ok(matched) => matched,
            Err(_err_str) => {
                return Ok(());
            }
        };

        self.engine.update_progress(target, Some(matched));
        self.run_engine_commands::<Entry<C>>(&[]).await?;

        self.update_replication_metrics(target, matched);

        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn update_replication_metrics(&mut self, target: C::NodeId, matched: LogId<C::NodeId>) {
        tracing::debug!(%target, ?matched, "update_leader_metrics");

        if let Some(l) = &mut self.leader_data {
            l.replication_metrics.update(UpdateMatchedLogId { target, matched });
        } else {
            unreachable!("it has to be a leader!!!");
        }
        self.engine.metrics_flags.set_replication_changed()
    }

    /// If a message is sent by a previous server state but is received by current server state,
    /// it is a stale message and should be just ignored.
    fn does_server_state_count_match(&self, vote: Vote<C::NodeId>, msg: impl Display) -> bool {
        if vote != self.engine.state.vote {
            tracing::warn!(
                "vote changed: msg sent by: {:?}; curr: {}; ignore when ({})",
                vote,
                self.engine.state.vote,
                msg
            );
            false
        } else {
            true
        }
    }

    /// A replication streams requesting for snapshot info.
    ///
    /// The snapshot has to include `must_include`.
    #[tracing::instrument(level = "debug", skip(self, tx))]
    async fn handle_needs_snapshot(
        &mut self,
        must_include: Option<LogId<C::NodeId>>,
        tx: oneshot::Sender<Snapshot<C::NodeId, S::SnapshotData>>,
    ) -> Result<(), StorageError<C::NodeId>> {
        // Ensure snapshotting is configured, else do nothing.
        let threshold = match &self.config.snapshot_policy {
            SnapshotPolicy::LogsSinceLast(threshold) => *threshold,
        };

        // Check for existence of current snapshot.
        let current_snapshot_opt = self.storage.get_current_snapshot().await?;

        if let Some(snapshot) = current_snapshot_opt {
            if let Some(must_inc) = must_include {
                if snapshot.meta.last_log_id >= must_inc {
                    let _ = tx.send(snapshot);
                    return Ok(());
                }
            } else {
                // If snapshot exists, ensure its distance from the leader's last log index is <= half
                // of the configured snapshot threshold, else create a new snapshot.
                if snapshot_is_within_half_of_threshold(
                    &snapshot.meta.last_log_id.index,
                    &self.engine.state.last_log_id().unwrap_or_default().index,
                    &threshold,
                ) {
                    let _ = tx.send(snapshot);
                    return Ok(());
                }
            }
        }

        // Check if snapshot creation is already in progress. If so, we spawn a task to await its
        // completion (or cancellation), and respond to the replication stream. The repl stream
        // will wait for the completion and will then send another request to fetch the finished snapshot.
        // Else we just drop any other state and continue. Leaders never enter `Streaming` state.
        if let Some(SnapshotState::Snapshotting { handle, sender }) = self.snapshot_state.take() {
            let mut chan = sender.subscribe();
            tokio::spawn(
                async move {
                    let _ = chan.recv().await;
                    // TODO(xp): send another ReplicaEvent::NeedSnapshot to raft core
                    drop(tx);
                }
                .instrument(tracing::debug_span!("spawn-recv-and-drop")),
            );
            self.snapshot_state = Some(SnapshotState::Snapshotting { handle, sender });
            return Ok(());
        }

        // At this point, we just attempt to request a snapshot. Under normal circumstances, the
        // leader will always be keeping up-to-date with its snapshotting, and the latest snapshot
        // will always be found and this block will never even be executed.
        //
        // If this block is executed, and a snapshot is needed, the repl stream will submit another
        // request here shortly, and will hit the above logic where it will await the snapshot completion.
        //
        // If snapshot is too old, i.e., the distance from last_log_index is greater than half of snapshot threshold,
        // always force a snapshot creation.
        self.trigger_log_compaction_if_needed(true).await;
        Ok(())
    }
}

#[async_trait::async_trait]
impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftRuntime<C> for RaftCore<C, N, S> {
    async fn run_command<'e, Ent>(
        &mut self,
        input_ref_entries: &'e [Ent],
        cur: &mut usize,
        cmd: &Command<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>>
    where
        Ent: RaftLogId<C::NodeId> + Sync + Send + 'e,
        &'e Ent: Into<Entry<C>>,
    {
        // Run non-role-specific command.
        match cmd {
            Command::UpdateServerState { .. } => {
                // TODO: This is not used. server state is already set by the engine.
                // This only used for notifying that metrics changed and probably will be removed when Engine is
                // finished.
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
            Command::MoveInputCursorBy { n } => *cur += n,
            Command::SaveVote { vote } => {
                self.storage.save_vote(vote).await?;
            }
            Command::InstallElectionTimer { can_be_leader } => {
                self.set_next_election_time(*can_be_leader);
            }
            Command::RejectElection {} => {
                self.reject_election_for_a_while();
            }
            Command::PurgeLog { upto } => self.storage.purge_logs_upto(*upto).await?,
            Command::DeleteConflictLog { since } => {
                self.storage.delete_conflict_logs_since(*since).await?;
            }
            Command::BuildSnapshot { .. } => {}
            Command::SendVote { vote_req } => {
                self.spawn_parallel_vote_requests(vote_req).await;
            }
            Command::ReplicateCommitted { committed } => {
                if let Some(l) = &self.leader_data {
                    for node in l.nodes.values() {
                        let _ = node.repl_tx.send(UpdateReplication {
                            last_log_id: None,
                            committed: *committed,
                        });
                    }
                } else {
                    unreachable!("it has to be a leader!!!");
                }
            }
            Command::LeaderCommit { ref upto, .. } => {
                for i in self.engine.state.last_applied.next_index()..(upto.index + 1) {
                    self.leader_commit(i).await?;
                }
            }
            Command::FollowerCommit { upto, .. } => {
                self.apply_to_state_machine(upto.index).await?;
            }
            Command::ReplicateInputEntries { range } => {
                if let Some(last) = range.clone().last() {
                    self.replicate_entry(*input_ref_entries[last].get_log_id());
                }
            }
            Command::UpdateReplicationStreams { remove, add } => {
                for (node_id, _matched) in remove.iter() {
                    self.remove_replication(*node_id).await;
                }
                for (node_id, _matched) in add.iter() {
                    let state = self.spawn_replication_stream(*node_id).await;
                    if let Some(l) = &mut self.leader_data {
                        l.nodes.insert(*node_id, state);
                    } else {
                        unreachable!("it has to be a leader!!!");
                    }
                }
            }
            Command::UpdateMembership { .. } => {
                // TODO: not used
            }
        }

        Ok(())
    }
}
