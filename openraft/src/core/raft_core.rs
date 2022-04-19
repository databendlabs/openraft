use std::mem::swap;
use std::sync::Arc;

use futures::future::AbortHandle;
use futures::future::Abortable;
use rand::thread_rng;
use rand::Rng;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio::time::Instant;
use tracing::trace_span;
use tracing::Instrument;
use tracing::Span;

use crate::config::Config;
use crate::config::SnapshotPolicy;
use crate::core::CandidateState;
use crate::core::FollowerState;
use crate::core::LeaderState;
use crate::core::LearnerState;
use crate::core::ServerState;
use crate::core::SnapshotState;
use crate::core::SnapshotUpdate;
use crate::engine::Command;
use crate::engine::Engine;
use crate::entry::EntryRef;
use crate::error::Fatal;
use crate::error::ForwardToLeader;
use crate::error::InitializeError;
use crate::error::NotAllowed;
use crate::membership::EffectiveMembership;
use crate::metrics::RaftMetrics;
use crate::metrics::ReplicationMetrics;
use crate::raft::RaftMsg;
use crate::raft::RaftRespTx;
use crate::raft_types::LogIdOptionExt;
use crate::runtime::RaftRuntime;
use crate::storage::RaftSnapshotBuilder;
use crate::versioned::Versioned;
use crate::Entry;
use crate::LogId;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Update;

pub trait MetricsProvider<NID: NodeId> {
    /// The default impl for the non-leader state
    fn get_leader_metrics(&self) -> Option<&Versioned<ReplicationMetrics<NID>>> {
        None
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

    /// The node's current snapshot state.
    pub(crate) snapshot_state: Option<SnapshotState<S::SnapshotData>>,

    /// The log id upto which the current snapshot includes, inclusive, if a snapshot exists.
    ///
    /// This is primarily used in making a determination on when a compaction job needs to be triggered.
    pub(crate) snapshot_last_log_id: Option<LogId<C::NodeId>>,

    /// The last time a heartbeat was received.
    pub(crate) last_heartbeat: Option<Instant>,

    /// The duration until the next election timeout.
    pub(crate) next_election_timeout: Option<Instant>,

    tx_compaction: mpsc::Sender<SnapshotUpdate<C>>,
    pub(crate) rx_compaction: mpsc::Receiver<SnapshotUpdate<C>>,

    pub(crate) rx_api: mpsc::UnboundedReceiver<(RaftMsg<C, N, S>, Span)>,

    tx_metrics: watch::Sender<RaftMetrics<C>>,

    pub(crate) rx_shutdown: oneshot::Receiver<()>,
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    pub(crate) fn spawn(
        id: C::NodeId,
        config: Arc<Config>,
        network: N,
        storage: S,
        rx_api: mpsc::UnboundedReceiver<(RaftMsg<C, N, S>, Span)>,
        tx_metrics: watch::Sender<RaftMetrics<C>>,
        rx_shutdown: oneshot::Receiver<()>,
    ) -> JoinHandle<Result<(), Fatal<C::NodeId>>> {
        //

        let (tx_compaction, rx_compaction) = mpsc::channel(1);

        let this = Self {
            id,
            config,
            network,
            storage,

            engine: Engine::default(),

            snapshot_state: None,
            snapshot_last_log_id: None,
            last_heartbeat: None,
            next_election_timeout: None,

            tx_compaction,
            rx_compaction,

            rx_api,

            tx_metrics,

            rx_shutdown,
        };
        tokio::spawn(this.main().instrument(trace_span!("spawn").or_current()))
    }

    /// The main loop of the Raft protocol.
    #[tracing::instrument(level="trace", skip(self), fields(id=display(self.id), cluster=%self.config.cluster_name))]
    async fn main(mut self) -> Result<(), Fatal<C::NodeId>> {
        let res = self.do_main().await;
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

        let state = self.storage.get_initial_state().await?;

        // TODO(xp): this is not necessary.
        self.storage.save_vote(&state.vote).await?;

        self.engine = Engine::new(self.id, &state);

        self.engine.state.last_applied = state.last_applied;

        // NOTE: The commit index must be determined by a leader after
        // successfully committing a new log to the cluster.
        self.engine.state.committed = None;

        // Fetch the most recent snapshot in the system.
        if let Some(snapshot) = self.storage.get_current_snapshot().await? {
            self.snapshot_last_log_id = Some(snapshot.meta.last_log_id);
            self.engine.metrics_flags.set_data_changed();
        }

        let has_log = self.engine.state.last_log_id.is_some();
        let single = self.engine.state.effective_membership.is_single();
        let is_voter = self.engine.state.effective_membership.membership.is_member(&self.id);

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
            // a single-node cluster. Become leader.
            (HAS_LOG, SINGLE, IS_VOTER) => ServerState::Leader,

            // The initial state when a raft is created from empty store.
            (NO_LOG, SINGLE, IS_VOTER) => ServerState::Learner,

            // Otherwise it is Follower.
            (HAS_LOG, MULTI, IS_VOTER) => ServerState::Follower,

            (NO_LOG, MULTI, IS_VOTER) => ServerState::Follower, // impossible: no logs but there are other members.
        };

        if self.engine.state.server_state == ServerState::Follower {
            // Here we use a 30 second overhead on the initial next_election_timeout. This is because we need
            // to ensure that restarted nodes don't disrupt a stable cluster by timing out and driving up their
            // term before network communication is established.
            let inst =
                Instant::now() + Duration::from_millis(thread_rng().gen_range(1..3) * self.config.heartbeat_interval);
            self.next_election_timeout = Some(inst);
        }

        tracing::debug!("id={} target_state: {:?}", self.id, self.engine.state.server_state);

        // This is central loop of the system. The Raft core assumes a few different roles based
        // on cluster state. The Raft core will delegate control to the different state
        // controllers and simply awaits the delegated loop to return, which will only take place
        // if some error has been encountered, or if a state change is required.
        loop {
            match &self.engine.state.server_state {
                ServerState::Leader => LeaderState::new(self).run().await?,
                ServerState::Candidate => CandidateState::new(self).run().await?,
                ServerState::Follower => FollowerState::new(self).run().await?,
                ServerState::Learner => LearnerState::new(self).run().await?,
                ServerState::Shutdown => {
                    tracing::info!("node has shutdown");
                    return Ok(());
                }
            }
        }
    }

    #[tracing::instrument(level = "trace", skip(self, metrics_reporter))]
    pub fn report_metrics_if_needed(&self, metrics_reporter: &impl MetricsProvider<C::NodeId>) {
        if !self.engine.metrics_flags.changed() {
            return;
        }

        let leader_metrics = if self.engine.metrics_flags.leader {
            Update::Update(metrics_reporter.get_leader_metrics().cloned())
        } else {
            Update::AsIs
        };

        self.report_metrics(leader_metrics);
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
            last_log_index: self.engine.state.last_log_id.map(|id| id.index),
            last_applied: self.engine.state.last_applied,
            snapshot: self.snapshot_last_log_id,

            // --- cluster ---
            state: self.engine.state.server_state,
            current_leader: self.current_leader(),
            membership_config: self.engine.state.effective_membership.clone(),

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
            && !self.engine.state.effective_membership.membership.is_member(&self.id)
        {
            self.engine.state.server_state = ServerState::Learner;
        } else {
            self.engine.state.server_state = target_state;
        }
    }

    /// Get the next election timeout, generating a new value if not set.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn get_next_election_timeout(&mut self) -> Instant {
        match self.next_election_timeout {
            Some(inst) => inst,
            None => {
                let t = Duration::from_millis(self.config.new_rand_election_timeout());
                tracing::debug!("create election timeout after: {:?}", t);
                let inst = Instant::now() + t;
                self.next_election_timeout = Some(inst);
                inst
            }
        }
    }

    /// Set a value for the next election timeout.
    ///
    /// If `heartbeat=true`, then also update the value of `last_heartbeat`.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn update_next_election_timeout(&mut self, heartbeat: bool) {
        let now = Instant::now();

        let t = Duration::from_millis(self.config.new_rand_election_timeout());
        tracing::debug!("update election timeout after: {:?}", t);

        self.next_election_timeout = Some(now + t);
        if heartbeat {
            self.last_heartbeat = Some(now);
        }
    }

    /// Update the node's current membership config & save hard state.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn update_membership(&mut self, cfg: EffectiveMembership<C::NodeId>) {
        // If the given config does not contain this node's ID, it means one of the following:
        //
        // - the node is currently a learner and is replicating an old config to which it has
        // not yet been added.
        // - the node has been removed from the cluster. The parent application can observe the
        // transition to the learner state as a signal for when it is safe to shutdown a node
        // being removed.
        self.engine.state.effective_membership = Arc::new(cfg);
        if self.engine.state.effective_membership.membership.is_member(&self.id) {
            if self.engine.state.server_state == ServerState::Learner {
                // The node is a Learner and the new config has it configured as a normal member.
                // Transition to follower.
                self.set_target_state(ServerState::Follower);
            }
        } else {
            self.set_target_state(ServerState::Learner);
        }
    }

    /// Update the system's snapshot state based on the given data.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(crate) fn update_snapshot_state(&mut self, update: SnapshotUpdate<C>) {
        if let SnapshotUpdate::SnapshotComplete(log_id) = update {
            self.snapshot_last_log_id = Some(log_id);
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
        if Some(last_applied.index) < self.snapshot_last_log_id.index() {
            return;
        }

        if !force {
            // If we are below the threshold, then there is nothing to do.
            if self.engine.state.last_applied.next_index() - self.snapshot_last_log_id.next_index() < *threshold {
                return;
            }
        }

        // At this point, we are clear to begin a new compaction process.
        let mut builder = self.storage.get_snapshot_builder().await;
        let (handle, reg) = AbortHandle::new_pair();
        let (chan_tx, _) = broadcast::channel(1);
        let tx_compaction = self.tx_compaction.clone();
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
                            let _ = tx_compaction.try_send(SnapshotUpdate::SnapshotComplete(snapshot.meta.last_log_id));
                            // This will always succeed.
                            let _ = chan_tx.send(snapshot.meta.last_log_id.index);
                        }
                        Err(err) => {
                            tracing::error!({error=%err}, "error while generating snapshot");
                            let _ = tx_compaction.try_send(SnapshotUpdate::SnapshotFailed);
                        }
                    },
                    Err(_aborted) => {
                        let _ = tx_compaction.try_send(SnapshotUpdate::SnapshotFailed);
                    }
                }
            }
            .instrument(tracing::debug_span!("beginning new log compaction process")),
        );
    }

    /// Reject an init config request due to the Raft node being in a state which prohibits the request.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    pub(crate) fn reject_init_with_config(&self, tx: oneshot::Sender<Result<(), InitializeError<C::NodeId>>>) {
        let _ = tx.send(Err(InitializeError::NotAllowed(NotAllowed {
            last_log_id: self.engine.state.last_log_id,
            vote: self.engine.state.vote,
        })));
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
            Some(id) => self.engine.state.effective_membership.get_node(&id).cloned(),
        }
    }
}

#[tracing::instrument(level = "trace", skip(sto), fields(entries=%entries.summary()))]
pub(crate) async fn apply_to_state_machine<C, S>(
    sto: &mut S,
    entries: &[&Entry<C>],
    max_keep: u64,
) -> Result<Vec<C::R>, StorageError<C::NodeId>>
where
    C: RaftTypeConfig,
    S: RaftStorage<C>,
{
    tracing::debug!(entries=%entries.summary(), max_keep, "apply_to_state_machine");

    let last = entries.last().map(|x| x.log_id);

    if let Some(last_applied) = last {
        // TODO(xp): apply_to_state_machine should return the last applied
        let res = sto.apply_to_state_machine(entries).await?;
        purge_applied_logs(sto, &last_applied, max_keep).await?;
        Ok(res)
    } else {
        Ok(vec![])
    }
}

#[tracing::instrument(level = "trace", skip(sto))]
pub(crate) async fn purge_applied_logs<C, S>(
    sto: &mut S,
    last_applied: &LogId<C::NodeId>,
    max_keep: u64,
) -> Result<(), StorageError<C::NodeId>>
where
    C: RaftTypeConfig,
    S: RaftStorage<C>,
{
    // TODO(xp): periodically batch delete
    let end = last_applied.index + 1;
    let end = end.saturating_sub(max_keep);

    tracing::debug!(%last_applied, max_keep, delete_lt = end, "delete_applied_logs");

    if end == 0 {
        return Ok(());
    }

    let st = sto.get_log_state().await?;

    if st.last_log_id < Some(*last_applied) {
        sto.purge_logs_upto(*last_applied).await?;
        return Ok(());
    }

    // non applied logs are deleted. it is a bug.
    assert!(st.last_purged_log_id <= Some(*last_applied));

    if st.last_purged_log_id.index() >= Some(end - 1) {
        return Ok(());
    }

    let log_id = sto.get_log_id(end - 1).await?;
    sto.purge_logs_upto(log_id).await
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    // TODO:
    #[allow(dead_code)]
    async fn run_engine_commands<'p>(
        &mut self,
        input_entries: &[EntryRef<'p, C>],
    ) -> Result<(), StorageError<C::NodeId>> {
        let mut curr = 0;
        let mut commands = vec![];
        swap(&mut self.engine.commands, &mut commands);
        for cmd in commands {
            self.run_command(input_entries, &mut curr, &cmd).await?;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftRuntime<C> for RaftCore<C, N, S> {
    async fn run_command<'p>(
        &mut self,
        input_ref_entries: &[EntryRef<'p, C>],
        cur: &mut usize,
        cmd: &Command<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        // Run non-role-specific command.
        match cmd {
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
            Command::SaveVote { .. } => {}
            Command::PurgeAppliedLog { .. } => {}
            Command::DeleteConflictLog { .. } => {}
            Command::BuildSnapshot { .. } => {}
            Command::SendVote { .. } => {}
            Command::Commit { .. } => {}
            Command::ReplicateInputEntries { .. } => {
                unreachable!("leader specific command")
            }
            Command::UpdateMembership { .. } => {}
        }

        Ok(())
    }
}
