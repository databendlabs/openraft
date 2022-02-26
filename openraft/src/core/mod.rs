//! The core logic of a Raft node.

mod admin;
mod append_entries;
mod client;
mod install_snapshot;
pub(crate) mod replication;
#[cfg(test)]
mod replication_state_test;
mod vote;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::Arc;

use futures::future::AbortHandle;
use futures::future::Abortable;
use maplit::btreeset;
use rand::thread_rng;
use rand::Rng;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::sleep_until;
use tokio::time::Duration;
use tokio::time::Instant;
use tracing::trace_span;
use tracing::Instrument;
use tracing::Span;

use crate::config::Config;
use crate::config::SnapshotPolicy;
use crate::core::client::ClientRequestEntry;
use crate::error::AddLearnerError;
use crate::error::ExtractFatal;
use crate::error::Fatal;
use crate::error::ForwardToLeader;
use crate::error::InitializeError;
use crate::metrics::LeaderMetrics;
use crate::metrics::RaftMetrics;
use crate::raft::AddLearnerResponse;
use crate::raft::Entry;
use crate::raft::EntryPayload;
use crate::raft::RaftMsg;
use crate::raft::RaftRespTx;
use crate::raft::VoteResponse;
use crate::raft_types::LogIdOptionExt;
use crate::replication::ReplicaEvent;
use crate::replication::ReplicationStream;
use crate::storage::RaftSnapshotBuilder;
use crate::vote::Vote;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MessageSummary;
use crate::Node;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Update;

/// The currently active membership config.
///
/// It includes:
/// - the id of the log that sets this membership config,
/// - and the config.
///
/// An active config is just the last seen config in raft spec.
#[derive(Clone, Eq, Serialize, Deserialize)]
pub struct EffectiveMembership<C: RaftTypeConfig> {
    /// The id of the log that applies this membership config
    pub log_id: LogId<C>,

    pub membership: Membership<C>,

    /// Cache of union of all members
    all_members: BTreeSet<C::NodeId>,
}

impl<C: RaftTypeConfig> Debug for EffectiveMembership<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectiveMembership")
            .field("log_id", &self.log_id)
            .field("membership", &self.membership)
            .field("all_members", &self.all_members)
            .finish()
    }
}

impl<C: RaftTypeConfig> PartialEq for EffectiveMembership<C> {
    fn eq(&self, other: &Self) -> bool {
        self.log_id == other.log_id && self.membership == other.membership && self.all_members == other.all_members
    }
}

impl<C: RaftTypeConfig> EffectiveMembership<C> {
    pub fn new_initial(node_id: C::NodeId) -> Self {
        Self::new(LogId::new(LeaderId::default(), 0), Membership::new_initial(node_id))
    }

    pub fn new(log_id: LogId<C>, membership: Membership<C>) -> Self {
        let all_members = membership.all_members();
        Self {
            log_id,
            membership,
            all_members,
        }
    }

    pub(crate) fn all_members(&self) -> &BTreeSet<C::NodeId> {
        &self.all_members
    }

    pub(crate) fn all_learners(&self) -> &BTreeSet<C::NodeId> {
        self.membership.all_learners()
    }

    // TODO(xp): unused
    pub fn get_configs(&self) -> &Vec<BTreeSet<C::NodeId>> {
        self.membership.get_configs()
    }

    pub fn get_node(&self, node_id: C::NodeId) -> Option<&Node> {
        self.membership.get_node(node_id)
    }

    pub fn get_nodes(&self) -> Option<&BTreeMap<C::NodeId, Node>> {
        self.membership.get_nodes().as_ref()
    }
}

impl<C: RaftTypeConfig> MessageSummary for EffectiveMembership<C> {
    fn summary(&self) -> String {
        format!("{{log_id:{} membership:{}}}", self.log_id, self.membership.summary())
    }
}

/// The core type implementing the Raft protocol.
pub struct RaftCore<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    /// This node's ID.
    id: C::NodeId,

    /// This node's runtime config.
    config: Arc<Config>,

    /// The cluster's current membership configuration.
    effective_membership: Arc<EffectiveMembership<C>>,

    /// The `RaftNetworkFactory` implementation.
    network: N,

    /// The `RaftStorage` implementation.
    storage: S,

    /// The target state of the system.
    target_state: State,

    /// The log id of the last known committed entry.
    ///
    /// Committed means:
    /// - a log that is replicated to a quorum of the cluster and it is of the term of the leader.
    /// - A quorum could be a joint quorum.
    committed: Option<LogId<C>>,

    /// The log id of the highest log entry which has been applied to the local state machine.
    last_applied: Option<LogId<C>>,

    /// The vote state of this node.
    vote: Vote<C>,

    /// The last entry to be appended to the log.
    last_log_id: Option<LogId<C>>,

    /// The node's current snapshot state.
    snapshot_state: Option<SnapshotState<S::SnapshotData>>,

    /// The log id upto which the current snapshot includes, inclusive, if a snapshot exists.
    ///
    /// This is primarily used in making a determination on when a compaction job needs to be triggered.
    snapshot_last_log_id: Option<LogId<C>>,

    /// The last time a heartbeat was received.
    last_heartbeat: Option<Instant>,

    /// The duration until the next election timeout.
    next_election_timeout: Option<Instant>,

    tx_compaction: mpsc::Sender<SnapshotUpdate<C>>,
    rx_compaction: mpsc::Receiver<SnapshotUpdate<C>>,

    rx_api: mpsc::UnboundedReceiver<(RaftMsg<C>, Span)>,

    tx_metrics: watch::Sender<RaftMetrics<C>>,

    rx_shutdown: oneshot::Receiver<()>,
}

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    pub(crate) fn spawn(
        id: C::NodeId,
        config: Arc<Config>,
        network: N,
        storage: S,
        rx_api: mpsc::UnboundedReceiver<(RaftMsg<C>, Span)>,
        tx_metrics: watch::Sender<RaftMetrics<C>>,
        rx_shutdown: oneshot::Receiver<()>,
    ) -> JoinHandle<Result<(), Fatal<C>>> {
        //

        // TODO(xp): remove this.
        let membership = Membership::new_initial(id); // This is updated from storage in the main loop.
        let (tx_compaction, rx_compaction) = mpsc::channel(1);

        let this = Self {
            id,
            config,
            effective_membership: Arc::new(EffectiveMembership::new(LogId::default(), membership)),
            network,
            storage,
            target_state: State::Follower,
            committed: None,
            last_applied: None,
            vote: Vote::default(),
            last_log_id: None,
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
    async fn main(mut self) -> Result<(), Fatal<C>> {
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
    async fn do_main(&mut self) -> Result<(), Fatal<C>> {
        tracing::debug!("raft node is initializing");

        let state = self.storage.get_initial_state().await?;

        // TODO(xp): this is not necessary.
        self.storage.save_vote(&state.vote).await?;

        self.last_log_id = state.last_log_id;
        self.vote = state.vote;
        self.effective_membership =
            Arc::new(state.last_membership.unwrap_or_else(|| EffectiveMembership::new_initial(self.id)));
        self.last_applied = state.last_applied;

        // NOTE: The commit index must be determined by a leader after
        // successfully committing a new log to the cluster.
        self.committed = None;

        // Fetch the most recent snapshot in the system.
        if let Some(snapshot) = self.storage.get_current_snapshot().await? {
            self.snapshot_last_log_id = Some(snapshot.meta.last_log_id);
            self.report_metrics(Update::AsIs);
        }

        let has_log = if self.last_log_id.is_some() {
            "has_log"
        } else {
            "no_log"
        };

        let single = if self.effective_membership.membership.all_members().len() == 1 {
            "single"
        } else {
            "multi"
        };

        let is_voter = if self.effective_membership.membership.is_member(&self.id) {
            "voter"
        } else {
            "learner"
        };

        self.target_state = match (has_log, single, is_voter) {
            // A restarted raft that already received some logs but was not yet added to a cluster.
            // It should remain in Learner state, not Follower.
            ("has_log", "single", "learner") => State::Learner,
            ("has_log", "multi", "learner") => State::Learner,

            ("no_log", "single", "learner") => State::Learner, // impossible: no logs but there are other members.
            ("no_log", "multi", "learner") => State::Learner,  // impossible: no logs but there are other members.

            // If this is the only configured member and there is live state, then this is
            // a single-node cluster. Become leader.
            ("has_log", "single", "voter") => State::Leader,

            // The initial state when a raft is created from empty store.
            ("no_log", "single", "voter") => State::Learner,

            // Otherwise it is Follower.
            ("has_log", "multi", "voter") => State::Follower,

            ("no_log", "multi", "voter") => State::Follower, // impossible: no logs but there are other members.

            _ => {
                panic!("invalid state: {}, {}, {}", has_log, single, is_voter);
            }
        };

        if self.target_state == State::Follower {
            // Here we use a 30 second overhead on the initial next_election_timeout. This is because we need
            // to ensure that restarted nodes don't disrupt a stable cluster by timing out and driving up their
            // term before network communication is established.
            let inst =
                Instant::now() + Duration::from_millis(thread_rng().gen_range(1..3) * self.config.heartbeat_interval);
            self.next_election_timeout = Some(inst);
        }

        tracing::debug!("id={} target_state: {:?}", self.id, self.target_state);

        // This is central loop of the system. The Raft core assumes a few different roles based
        // on cluster state. The Raft core will delegate control to the different state
        // controllers and simply awaits the delegated loop to return, which will only take place
        // if some error has been encountered, or if a state change is required.
        loop {
            match &self.target_state {
                State::Leader => LeaderState::new(self).run().await?,
                State::Candidate => CandidateState::new(self).run().await?,
                State::Follower => FollowerState::new(self).run().await?,
                State::Learner => LearnerState::new(self).run().await?,
                State::Shutdown => {
                    tracing::info!("node has shutdown");
                    return Ok(());
                }
            }
        }
    }

    /// Report a metrics payload on the current state of the Raft node.
    #[tracing::instrument(level = "trace", skip(self))]
    fn report_metrics(&mut self, leader_metrics: Update<Option<&Arc<LeaderMetrics<C>>>>) {
        let leader_metrics = match leader_metrics {
            Update::Update(v) => v.cloned(),
            Update::AsIs => self.tx_metrics.borrow().leader_metrics.clone(),
        };

        let m = RaftMetrics {
            running_state: Ok(()),

            id: self.id,
            state: self.target_state,
            current_term: self.vote.term,
            last_log_index: self.last_log_id.map(|id| id.index),
            last_applied: self.last_applied,
            current_leader: self.current_leader(),
            membership_config: self.effective_membership.clone(),
            snapshot: self.snapshot_last_log_id,
            leader_metrics,
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
    async fn save_vote(&mut self) -> Result<(), StorageError<C>> {
        self.storage.save_vote(&self.vote).await
    }

    /// Update core's target state, ensuring all invariants are upheld.
    #[tracing::instrument(level = "trace", skip(self), fields(id=display(self.id)))]
    fn set_target_state(&mut self, target_state: State) {
        tracing::debug!(id = display(self.id), ?target_state, "set_target_state");

        if target_state == State::Follower && !self.effective_membership.membership.is_member(&self.id) {
            self.target_state = State::Learner;
        } else {
            self.target_state = target_state;
        }
    }

    /// Get the next election timeout, generating a new value if not set.
    #[tracing::instrument(level = "trace", skip(self))]
    fn get_next_election_timeout(&mut self) -> Instant {
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
    fn update_next_election_timeout(&mut self, heartbeat: bool) {
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
    fn update_membership(&mut self, cfg: EffectiveMembership<C>) {
        // If the given config does not contain this node's ID, it means one of the following:
        //
        // - the node is currently a learner and is replicating an old config to which it has
        // not yet been added.
        // - the node has been removed from the cluster. The parent application can observe the
        // transition to the learner state as a signal for when it is safe to shutdown a node
        // being removed.
        self.effective_membership = Arc::new(cfg);
        if self.effective_membership.membership.is_member(&self.id) {
            if self.target_state == State::Learner {
                // The node is a Learner and the new config has it configured as a normal member.
                // Transition to follower.
                self.set_target_state(State::Follower);
            }
        } else {
            self.set_target_state(State::Learner);
        }
    }

    /// Update the system's snapshot state based on the given data.
    #[tracing::instrument(level = "trace", skip(self))]
    fn update_snapshot_state(&mut self, update: SnapshotUpdate<C>) {
        if let SnapshotUpdate::SnapshotComplete(log_id) = update {
            self.snapshot_last_log_id = Some(log_id);
            self.report_metrics(Update::AsIs);
        }
        // If snapshot state is anything other than streaming, then drop it.
        if let Some(state @ SnapshotState::Streaming { .. }) = self.snapshot_state.take() {
            self.snapshot_state = Some(state);
        }
    }

    /// Trigger a log compaction (snapshot) job if needed.
    /// If force is True, it will skip the threshold check and start creating snapshot as demanded.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(self) async fn trigger_log_compaction_if_needed(&mut self, force: bool) {
        if self.snapshot_state.is_some() {
            return;
        }
        let SnapshotPolicy::LogsSinceLast(threshold) = &self.config.snapshot_policy;

        let last_applied = match self.last_applied {
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
            if self.last_applied.next_index() - self.snapshot_last_log_id.next_index() < *threshold {
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
                            let _ = chan_tx.send(snapshot.meta.last_log_id.index); // This will always succeed.
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
    fn reject_init_with_config(&self, tx: oneshot::Sender<Result<(), InitializeError<C>>>) {
        let _ = tx.send(Err(InitializeError::NotAllowed));
    }

    /// Reject a request due to the Raft node being in a state which prohibits the request.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    fn reject_with_forward_to_leader<T, E>(&self, tx: RaftRespTx<T, E>)
    where E: From<ForwardToLeader<C>> {
        let l = self.current_leader();
        let err = ForwardToLeader {
            leader_id: l,
            leader_node: self.get_leader_node(l),
        };

        let _ = tx.send(Err(err.into()));
    }

    #[tracing::instrument(level = "debug", skip(self, payload))]
    pub(super) async fn append_payload_to_log(
        &mut self,
        payload: EntryPayload<C>,
    ) -> Result<Entry<C>, StorageError<C>> {
        let log_id = LogId::new(self.vote.leader_id(), self.last_log_id.next_index());

        let entry = Entry { log_id, payload };
        self.storage.append_to_log(&[&entry]).await?;

        tracing::debug!("append log: {}", entry.summary());
        self.last_log_id = Some(log_id);

        if let EntryPayload::Membership(mem) = &entry.payload {
            self.effective_membership = Arc::new(EffectiveMembership::new(entry.log_id, mem.clone()));
        }

        Ok(entry)
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub fn current_leader(&self) -> Option<C::NodeId> {
        if !self.vote.committed {
            return None;
        }

        let id = self.vote.node_id;

        if id == self.id {
            if self.target_state == State::Leader {
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
            Some(id) => self.effective_membership.get_node(id).cloned(),
        }
    }
}

#[tracing::instrument(level = "trace", skip(sto), fields(entries=%entries.summary()))]
async fn apply_to_state_machine<C, S>(
    sto: &mut S,
    entries: &[&Entry<C>],
    max_keep: u64,
) -> Result<Vec<C::R>, StorageError<C>>
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
async fn purge_applied_logs<C, S>(sto: &mut S, last_applied: &LogId<C>, max_keep: u64) -> Result<(), StorageError<C>>
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

/// The current snapshot state of the Raft node.
pub(self) enum SnapshotState<S> {
    /// The Raft node is compacting itself.
    Snapshotting {
        /// A handle to abort the compaction process early if needed.
        handle: AbortHandle,
        /// A sender for notifiying any other tasks of the completion of this compaction.
        sender: broadcast::Sender<u64>,
    },
    /// The Raft node is streaming in a snapshot from the leader.
    Streaming {
        /// The offset of the last byte written to the snapshot.
        offset: u64,
        /// The ID of the snapshot being written.
        id: String,
        /// A handle to the snapshot writer.
        snapshot: Box<S>,
    },
}

/// An update on a snapshot creation process.
#[derive(Debug)]
pub(self) enum SnapshotUpdate<C: RaftTypeConfig> {
    /// Snapshot creation has finished successfully and covers the given index.
    SnapshotComplete(LogId<C>),
    /// Snapshot creation failed.
    SnapshotFailed,
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/// All possible states of a Raft node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    /// The node is completely passive; replicating entries, but neither voting nor timing out.
    Learner,
    /// The node is replicating logs from the leader.
    Follower,
    /// The node is campaigning to become the cluster leader.
    Candidate,
    /// The node is the Raft cluster leader.
    Leader,
    /// The Raft node is shutting down.
    Shutdown,
}

impl State {
    /// Check if currently in learner state.
    pub fn is_learner(&self) -> bool {
        matches!(self, Self::Learner)
    }

    /// Check if currently in follower state.
    pub fn is_follower(&self) -> bool {
        matches!(self, Self::Follower)
    }

    /// Check if currently in candidate state.
    pub fn is_candidate(&self) -> bool {
        matches!(self, Self::Candidate)
    }

    /// Check if currently in leader state.
    pub fn is_leader(&self) -> bool {
        matches!(self, Self::Leader)
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/// Volatile state specific to the Raft leader.
struct LeaderState<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    pub(super) core: &'a mut RaftCore<C, N, S>,

    /// A mapping of node IDs the replication state of the target node.
    pub(super) nodes: BTreeMap<C::NodeId, ReplicationState<C>>,

    /// The metrics about a leader
    pub leader_metrics: Arc<LeaderMetrics<C>>,

    /// The stream of events coming from replication streams.
    pub(super) replication_rx: mpsc::UnboundedReceiver<(ReplicaEvent<C, S::SnapshotData>, Span)>,

    /// The cloneable sender channel for replication stream events.
    pub(super) replication_tx: mpsc::UnboundedSender<(ReplicaEvent<C, S::SnapshotData>, Span)>,

    /// A buffer of client requests which have been appended locally and are awaiting to be committed to the cluster.
    pub(super) awaiting_committed: Vec<ClientRequestEntry<C>>,
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LeaderState<'a, C, N, S> {
    /// Create a new instance.
    pub(self) fn new(core: &'a mut RaftCore<C, N, S>) -> Self {
        let (replication_tx, replication_rx) = mpsc::unbounded_channel();
        Self {
            core,
            nodes: BTreeMap::new(),
            leader_metrics: Arc::new(LeaderMetrics::default()),
            replication_tx,
            replication_rx,
            awaiting_committed: Vec::new(),
        }
    }

    /// Transition to the Raft leader state.
    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id), raft_state="leader"))]
    pub(self) async fn run(mut self) -> Result<(), Fatal<C>> {
        // Setup state as leader.
        self.core.last_heartbeat = None;
        self.core.next_election_timeout = None;
        self.core.vote.commit();

        // Spawn replication streams for followers and learners.
        let targets = self
            .core
            .effective_membership
            .all_members()
            .iter()
            .filter(|elem| *elem != &self.core.id)
            .chain(self.core.effective_membership.all_learners().iter())
            .cloned()
            .collect::<Vec<_>>();

        for target in targets {
            let state = self.spawn_replication_stream(target, None).await;
            self.nodes.insert(target, state);
        }

        self.leader_report_metrics();

        self.commit_initial_leader_entry().await?;

        self.leader_loop().await?;

        Ok(())
    }

    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id)))]
    pub(self) async fn leader_loop(mut self) -> Result<(), Fatal<C>> {
        loop {
            if !self.core.target_state.is_leader() {
                tracing::info!("id={} state becomes: {:?}", self.core.id, self.core.target_state);

                // implicit drop replication_rx
                // notify to all nodes DO NOT send replication event any more.
                return Ok(());
            }

            let span = tracing::debug_span!("CHrx:LeaderState");
            let _ent = span.enter();

            tokio::select! {
                Some((msg,span)) = self.core.rx_api.recv() => {
                    self.handle_msg(msg).instrument(span).await?;
                },

                Some(update) = self.core.rx_compaction.recv() => {
                    tracing::info!("leader recv from rx_compaction: {:?}", update);
                    self.core.update_snapshot_state(update);
                }

                Some((event, span)) = self.replication_rx.recv() => {
                    tracing::info!("leader recv from replication_rx: {:?}", event.summary());
                    self.handle_replica_event(event).instrument(span).await?;
                }

                Ok(_) = &mut self.core.rx_shutdown => {
                    tracing::info!("leader recv from rx_shudown");
                    self.core.set_target_state(State::Shutdown);
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = "leader", id=display(self.core.id)))]
    pub async fn handle_msg(&mut self, msg: RaftMsg<C>) -> Result<(), Fatal<C>> {
        tracing::debug!("recv from rx_api: {}", msg.summary());

        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                let res = self.core.handle_append_entries_request(rpc).await.extract_fatal()?;
                let _ = tx.send(res);
            }
            RaftMsg::RequestVote { rpc, tx } => {
                let res = self.core.handle_vote_request(rpc).await.extract_fatal()?;
                let _ = tx.send(res);
            }
            RaftMsg::InstallSnapshot { rpc, tx } => {
                let res = self.core.handle_install_snapshot_request(rpc).await.extract_fatal()?;
                let _ = tx.send(res);
            }
            RaftMsg::ClientReadRequest { tx } => {
                self.handle_client_read_request(tx).await;
            }
            RaftMsg::ClientWriteRequest { rpc, tx } => {
                self.handle_client_write_request(rpc, tx).await?;
            }
            RaftMsg::Initialize { tx, .. } => {
                self.core.reject_init_with_config(tx);
            }
            RaftMsg::AddLearner { id, node, tx, blocking } => {
                self.add_learner(id, node, tx, blocking).await;
            }
            RaftMsg::ChangeMembership {
                members,
                blocking,
                turn_to_learner,
                tx,
            } => {
                self.change_membership(members, blocking, turn_to_learner, tx).await?;
            }
        };

        Ok(())
    }

    /// Report metrics with leader specific states.
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn leader_report_metrics(&mut self) {
        self.core.report_metrics(Update::Update(Some(&self.leader_metrics)));
    }
}

/// A struct tracking the state of a replication stream from the perspective of the Raft actor.
struct ReplicationState<C: RaftTypeConfig> {
    pub matched: Option<LogId<C>>,
    pub remove_since: Option<u64>,
    pub repl_stream: ReplicationStream<C>,

    /// The response channel to use for when this node has successfully synced with the cluster.
    pub tx: Option<RaftRespTx<AddLearnerResponse<C>, AddLearnerError<C>>>,
}

impl<C: RaftTypeConfig> MessageSummary for ReplicationState<C> {
    fn summary(&self) -> String {
        format!(
            "matched: {:?}, remove_after_commit: {:?}",
            self.matched, self.remove_since
        )
    }
}

impl<C: RaftTypeConfig> ReplicationState<C> {
    // TODO(xp): make this a method of Config?

    /// Return true if the distance behind last_log_id is smaller than the threshold to join.
    pub fn is_line_rate(&self, last_log_id: &Option<LogId<C>>, config: &Config) -> bool {
        is_matched_upto_date(&self.matched, last_log_id, config)
    }
}

pub fn is_matched_upto_date<C: RaftTypeConfig>(
    matched: &Option<LogId<C>>,
    last_log_id: &Option<LogId<C>>,
    config: &Config,
) -> bool {
    let my_index = matched.next_index();
    let distance = last_log_id.next_index().saturating_sub(my_index);
    distance <= config.replication_lag_threshold
}

/// Volatile state specific to a Raft node in candidate state.
struct CandidateState<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    core: &'a mut RaftCore<C, N, S>,

    /// Ids of the nodes that has granted our vote request.
    granted: BTreeSet<C::NodeId>,
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> CandidateState<'a, C, N, S> {
    pub(self) fn new(core: &'a mut RaftCore<C, N, S>) -> Self {
        Self {
            core,
            granted: btreeset! {},
        }
    }

    /// Run the candidate loop.
    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id), raft_state="candidate"))]
    pub(self) async fn run(mut self) -> Result<(), Fatal<C>> {
        // Each iteration of the outer loop represents a new term.

        loop {
            if !self.core.target_state.is_candidate() {
                return Ok(());
            }

            // Setup new term.
            self.core.update_next_election_timeout(false); // Generates a new rand value within range.

            self.core.vote = Vote::new(self.core.vote.term + 1, self.core.id);

            self.core.save_vote().await?;
            self.core.report_metrics(Update::Update(None));

            // vote for itself.
            self.handle_vote_response(
                VoteResponse {
                    vote: self.core.vote,
                    vote_granted: true,
                    last_log_id: self.core.last_log_id,
                },
                self.core.id,
            )
            .await?;
            if !self.core.target_state.is_candidate() {
                return Ok(());
            }

            // Send RPCs to all members in parallel.
            let mut pending_votes = self.spawn_parallel_vote_requests().await;

            // Inner processing loop for this Raft state.
            loop {
                if !self.core.target_state.is_candidate() {
                    return Ok(());
                }
                let timeout_fut = sleep_until(self.core.get_next_election_timeout());

                let span = tracing::debug_span!("CHrx:CandidateState");
                let _ent = span.enter();

                tokio::select! {
                    _ = timeout_fut => break, // This election has timed-out. Break to outer loop, which starts a new term.

                    Some((res, peer)) = pending_votes.recv() => {
                        self.handle_vote_response(res, peer).await?;
                    },

                    Some((msg,span)) = self.core.rx_api.recv() => {
                        self.handle_msg(msg).instrument(span).await?;
                    },

                    Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),

                    Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(State::Shutdown),
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = "candidate", id=display(self.core.id)))]
    pub async fn handle_msg(&mut self, msg: RaftMsg<C>) -> Result<(), Fatal<C>> {
        tracing::debug!("recv from rx_api: {}", msg.summary());
        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                let _ = tx.send(self.core.handle_append_entries_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::RequestVote { rpc, tx } => {
                let _ = tx.send(self.core.handle_vote_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::InstallSnapshot { rpc, tx } => {
                let _ = tx.send(self.core.handle_install_snapshot_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::ClientReadRequest { tx } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ClientWriteRequest { rpc: _, tx } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::Initialize { tx, .. } => {
                self.core.reject_init_with_config(tx);
            }
            RaftMsg::AddLearner { tx, .. } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ChangeMembership { tx, .. } => {
                self.core.reject_with_forward_to_leader(tx);
            }
        };
        Ok(())
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/// Volatile state specific to a Raft node in follower state.
pub struct FollowerState<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    core: &'a mut RaftCore<C, N, S>,
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> FollowerState<'a, C, N, S> {
    pub(self) fn new(core: &'a mut RaftCore<C, N, S>) -> Self {
        Self { core }
    }

    /// Run the follower loop.
    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id), raft_state="follower"))]
    pub(self) async fn run(mut self) -> Result<(), Fatal<C>> {
        self.core.report_metrics(Update::Update(None));

        loop {
            if !self.core.target_state.is_follower() {
                return Ok(());
            }

            let election_timeout = sleep_until(self.core.get_next_election_timeout()); // Value is updated as heartbeats are received.

            tokio::select! {
                // If an election timeout is hit, then we need to transition to candidate.
                _ = election_timeout => {
                    tracing::debug!("timeout to recv a event, change to CandidateState");
                    self.core.set_target_state(State::Candidate)
                },

                Some((msg,span)) = self.core.rx_api.recv() => {
                    self.handle_msg(msg).instrument(span).await?;
                },

                Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),

                Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(State::Shutdown),
            }
        }
    }

    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = "follower", id=display(self.core.id)))]
    pub(crate) async fn handle_msg(&mut self, msg: RaftMsg<C>) -> Result<(), Fatal<C>> {
        tracing::debug!("recv from rx_api: {}", msg.summary());

        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                let _ = tx.send(self.core.handle_append_entries_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::RequestVote { rpc, tx } => {
                let _ = tx.send(self.core.handle_vote_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::InstallSnapshot { rpc, tx } => {
                let _ = tx.send(self.core.handle_install_snapshot_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::ClientReadRequest { tx } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ClientWriteRequest { rpc: _, tx } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::Initialize { tx, .. } => {
                self.core.reject_init_with_config(tx);
            }
            RaftMsg::AddLearner { tx, .. } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ChangeMembership { tx, .. } => {
                self.core.reject_with_forward_to_leader(tx);
            }
        };
        Ok(())
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/// Volatile state specific to a Raft node in learner state.
pub struct LearnerState<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    core: &'a mut RaftCore<C, N, S>,
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LearnerState<'a, C, N, S> {
    pub(self) fn new(core: &'a mut RaftCore<C, N, S>) -> Self {
        Self { core }
    }

    /// Run the learner loop.
    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id), raft_state="learner"))]
    pub(self) async fn run(mut self) -> Result<(), Fatal<C>> {
        self.core.report_metrics(Update::Update(None));

        loop {
            if !self.core.target_state.is_learner() {
                return Ok(());
            }

            let span = tracing::debug_span!("CHrx:LearnerState");
            let _ent = span.enter();

            tokio::select! {
                Some((msg,span)) = self.core.rx_api.recv() => {
                    self.handle_msg(msg).instrument(span).await?;
                },

                Some(update) = self.core.rx_compaction.recv() => {
                    self.core.update_snapshot_state(update);
                },

                Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(State::Shutdown),
            }
        }
    }

    // TODO(xp): define a handle_msg method in RaftCore that decides what to do by current State.
    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = "learner", id=display(self.core.id)))]
    pub(crate) async fn handle_msg(&mut self, msg: RaftMsg<C>) -> Result<(), Fatal<C>> {
        tracing::debug!("recv from rx_api: {}", msg.summary());

        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                let _ = tx.send(self.core.handle_append_entries_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::RequestVote { rpc, tx } => {
                let _ = tx.send(self.core.handle_vote_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::InstallSnapshot { rpc, tx } => {
                let _ = tx.send(self.core.handle_install_snapshot_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::ClientReadRequest { tx } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ClientWriteRequest { rpc: _, tx } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::Initialize { members, tx } => {
                let _ = tx.send(self.handle_init_with_config(members).await);
            }
            RaftMsg::AddLearner { tx, .. } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ChangeMembership { tx, .. } => {
                self.core.reject_with_forward_to_leader(tx);
            }
        };
        Ok(())
    }
}
