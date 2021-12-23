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
use std::sync::Arc;

use futures::future::AbortHandle;
use futures::future::Abortable;
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
use tracing::Span;
use tracing_futures::Instrument;

use crate::config::Config;
use crate::config::SnapshotPolicy;
use crate::core::client::ClientRequestEntry;
use crate::error::AddNonVoterError;
use crate::error::ClientReadError;
use crate::error::ClientWriteError;
use crate::error::ForwardToLeader;
use crate::error::InitializeError;
use crate::error::RaftError;
use crate::error::RaftResult;
use crate::metrics::LeaderMetrics;
use crate::metrics::RaftMetrics;
use crate::raft::AddNonVoterResponse;
use crate::raft::ClientWriteRequest;
use crate::raft::ClientWriteResponse;
use crate::raft::Entry;
use crate::raft::EntryPayload;
use crate::raft::MembershipConfig;
use crate::raft::RaftMsg;
use crate::raft::RaftRespTx;
use crate::replication::RaftEvent;
use crate::replication::ReplicaEvent;
use crate::replication::ReplicationStream;
use crate::storage::HardState;
use crate::AppData;
use crate::AppDataResponse;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
use crate::RaftNetwork;
use crate::RaftStorage;
use crate::StorageError;
use crate::Update;

/// The currently active membership config.
///
/// It includes:
/// - the id of the log that sets this membership config,
/// - and the config.
///
/// An active config is just the last seen config in raft spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveMembership {
    /// The id of the log that applies this membership config
    pub log_id: LogId,

    pub membership: MembershipConfig,
}

/// The core type implementing the Raft protocol.
pub struct RaftCore<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    /// This node's ID.
    id: NodeId,
    /// This node's runtime config.
    config: Arc<Config>,

    /// The cluster's current membership configuration.
    membership: EffectiveMembership,

    /// The `RaftNetwork` implementation.
    network: Arc<N>,

    /// The `RaftStorage` implementation.
    storage: Arc<S>,

    /// The target state of the system.
    target_state: State,

    /// The index of the highest log entry known to be committed cluster-wide.
    ///
    /// The definition of a committed log is that the leader which has created the log has
    /// successfully replicated the log to a majority of the cluster. This value is updated via
    /// AppendEntries RPC from the leader, or if a node is the leader, it will update this value
    /// as new entries have been successfully replicated to a majority of the cluster.
    ///
    /// Is initialized to 0, and increases monotonically. This is always based on the leader's
    /// commit index which is communicated to other members via the AppendEntries protocol.
    commit_index: u64,

    /// The log id of the highest log entry which has been applied to the local state machine.
    ///
    /// Is initialized to 0,0 for a pristine node; else, for nodes with existing state it is
    /// is initialized to the value returned from the `RaftStorage::get_initial_state` on startup.
    /// This value increases following the `commit_index` as logs are applied to the state
    /// machine (via the storage interface).
    last_applied: LogId,

    /// The current term.
    ///
    /// Is initialized to 0 on first boot, and increases monotonically. This is normally based on
    /// the leader's term which is communicated to other members via the AppendEntries protocol,
    /// but this may also be incremented when a follower becomes a candidate.
    current_term: u64,
    /// The ID of the current leader of the Raft cluster.
    current_leader: Option<NodeId>,
    /// The ID of the candidate which received this node's vote for the current term.
    ///
    /// Each server will vote for at most one candidate in a given term, on a
    /// first-come-first-served basis. See §5.4.1 for additional restriction on votes.
    voted_for: Option<NodeId>,

    /// The last entry to be appended to the log.
    last_log_id: LogId,

    /// The node's current snapshot state.
    snapshot_state: Option<SnapshotState<S::SnapshotData>>,

    /// The log id upto which the current snapshot includes, inclusive, if a snapshot exists.
    ///
    /// This is primarily used in making a determination on when a compaction job needs to be triggered.
    snapshot_last_log_id: LogId,

    /// A bool indicating if this system has performed its initial replication of
    /// outstanding entries to the state machine.
    has_completed_initial_replication_to_sm: bool,

    /// The last time a heartbeat was received.
    last_heartbeat: Option<Instant>,
    /// The duration until the next election timeout.
    next_election_timeout: Option<Instant>,

    tx_compaction: mpsc::Sender<SnapshotUpdate>,
    rx_compaction: mpsc::Receiver<SnapshotUpdate>,

    rx_api: mpsc::UnboundedReceiver<(RaftMsg<D, R>, Span)>,
    tx_metrics: watch::Sender<RaftMetrics>,
    rx_shutdown: oneshot::Receiver<()>,
}

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> RaftCore<D, R, N, S> {
    pub(crate) fn spawn(
        id: NodeId,
        config: Arc<Config>,
        network: Arc<N>,
        storage: Arc<S>,
        rx_api: mpsc::UnboundedReceiver<(RaftMsg<D, R>, Span)>,
        tx_metrics: watch::Sender<RaftMetrics>,
        rx_shutdown: oneshot::Receiver<()>,
    ) -> JoinHandle<RaftResult<()>> {
        let membership = MembershipConfig::new_initial(id); // This is updated from storage in the main loop.
        let (tx_compaction, rx_compaction) = mpsc::channel(1);
        let this = Self {
            id,
            config,
            membership: EffectiveMembership {
                log_id: LogId::default(),
                membership,
            },
            network,
            storage,
            target_state: State::Follower,
            commit_index: 0,
            last_applied: LogId { term: 0, index: 0 },
            current_term: 0,
            current_leader: None,
            voted_for: None,
            last_log_id: LogId { term: 0, index: 0 },
            snapshot_state: None,
            snapshot_last_log_id: LogId { term: 0, index: 0 },
            has_completed_initial_replication_to_sm: false,
            last_heartbeat: None,
            next_election_timeout: None,
            tx_compaction,
            rx_compaction,
            rx_api,
            tx_metrics,
            rx_shutdown,
        };
        tokio::spawn(this.main())
    }

    /// The main loop of the Raft protocol.
    #[tracing::instrument(level="debug", skip(self), fields(id=self.id, cluster=%self.config.cluster_name))]
    async fn main(mut self) -> RaftResult<()> {
        tracing::debug!("raft node is initializing");

        let state = self.storage.get_initial_state().await.map_err(|err| self.map_storage_error(err))?;
        self.last_log_id = state.last_log_id;
        self.current_term = state.hard_state.current_term;
        self.voted_for = state.hard_state.voted_for;
        self.membership = state.last_membership.clone();
        self.last_applied = state.last_applied;
        // NOTE: this is repeated here for clarity. It is unsafe to initialize the node's commit
        // index to any other value. The commit index must be determined by a leader after
        // successfully committing a new log to the cluster.
        self.commit_index = 0;

        // Fetch the most recent snapshot in the system.
        if let Some(snapshot) = self.storage.get_current_snapshot().await.map_err(|err| self.map_storage_error(err))? {
            self.snapshot_last_log_id = snapshot.meta.last_log_id;
            self.report_metrics(Update::Ignore);
        }

        let has_log = self.last_log_id.index != u64::MIN;
        let single = self.membership.membership.members.len() == 1;
        let is_voter = self.membership.membership.contains(&self.id);

        self.target_state = match (has_log, single, is_voter) {
            // A restarted raft that already received some logs but was not yet added to a cluster.
            // It should remain in NonVoter state, not Follower.
            (true, true, false) => State::NonVoter,
            (true, false, false) => State::NonVoter,

            (false, true, false) => State::NonVoter, // impossible: no logs but there are other members.
            (false, false, false) => State::NonVoter, // impossible: no logs but there are other members.

            // If this is the only configured member and there is live state, then this is
            // a single-node cluster. Become leader.
            (true, true, true) => State::Leader,

            // The initial state when a raft is created from empty store.
            (false, true, true) => State::NonVoter,

            // Otherwise it is Follower.
            (true, false, true) => State::Follower,

            (false, false, true) => State::Follower, // impossible: no logs but there are other members.
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
                State::Leader => LeaderState::new(&mut self).run().await?,
                State::Candidate => CandidateState::new(&mut self).run().await?,
                State::Follower => FollowerState::new(&mut self).run().await?,
                State::NonVoter => NonVoterState::new(&mut self).run().await?,
                State::Shutdown => {
                    tracing::info!("node has shutdown");
                    return Ok(());
                }
            }
        }
    }

    /// Report a metrics payload on the current state of the Raft node.
    #[tracing::instrument(level = "debug", skip(self))]
    fn report_metrics(&mut self, leader_metrics: Update<Option<&LeaderMetrics>>) {
        let leader_metrics = match leader_metrics {
            Update::Update(v) => v.cloned(),
            Update::Ignore => self.tx_metrics.borrow().leader_metrics.clone(),
        };

        let res = self.tx_metrics.send(RaftMetrics {
            id: self.id,
            state: self.target_state,
            current_term: self.current_term,
            last_log_index: self.last_log_id.index,
            last_applied: self.last_applied.index,
            current_leader: self.current_leader,
            // TODO(xp): 111 metrics should also track the membership log id
            membership_config: self.membership.clone(),
            snapshot: self.snapshot_last_log_id,
            leader_metrics,
        });

        if let Err(err) = res {
            tracing::error!(error=%err, id=self.id, "error reporting metrics");
        }
    }

    /// Save the Raft node's current hard state to disk.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_hard_state(&mut self) -> RaftResult<()> {
        let hs = HardState {
            current_term: self.current_term,
            voted_for: self.voted_for,
        };
        self.storage.save_hard_state(&hs).await.map_err(|err| self.map_storage_error(err))
    }

    /// Update core's target state, ensuring all invariants are upheld.
    #[tracing::instrument(level = "trace", skip(self))]
    fn set_target_state(&mut self, target_state: State) {
        if target_state == State::Follower && !self.membership.membership.contains(&self.id) {
            self.target_state = State::NonVoter;
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

    /// Update the value of the `current_leader` property.
    #[tracing::instrument(level = "trace", skip(self))]
    fn update_current_leader(&mut self, update: UpdateCurrentLeader) {
        match update {
            UpdateCurrentLeader::ThisNode => {
                self.current_leader = Some(self.id);
            }
            UpdateCurrentLeader::OtherNode(target) => {
                self.current_leader = Some(target);
            }
            UpdateCurrentLeader::Unknown => {
                self.current_leader = None;
            }
        }
    }

    /// Encapsulate the process of updating the current term, as updating the `voted_for` state must also be updated.
    #[tracing::instrument(level = "trace", skip(self))]
    fn update_current_term(&mut self, new_term: u64, voted_for: Option<NodeId>) {
        if new_term > self.current_term {
            self.current_term = new_term;
            self.voted_for = voted_for;
        }
    }

    /// Trigger the shutdown sequence due to a non-recoverable error from the storage layer.
    ///
    /// This method assumes that a storage error observed here is non-recoverable. As such, the
    /// Raft node will be instructed to stop. If such behavior is not needed, then don't use this
    /// interface.
    #[tracing::instrument(level = "trace", skip(self))]
    fn map_fatal_storage_error(&mut self, err: anyhow::Error) -> RaftError {
        tracing::error!({error=?err, id=self.id}, "fatal storage error, shutting down");
        self.set_target_state(State::Shutdown);
        RaftError::RaftStorage(err)
    }

    fn map_storage_error(&mut self, err: StorageError) -> RaftError {
        tracing::error!({error=?err, id=self.id}, "fatal storage error, shutting down");
        self.set_target_state(State::Shutdown);
        RaftError::RaftStorage(err.into())
    }

    /// Update the node's current membership config & save hard state.
    #[tracing::instrument(level = "trace", skip(self))]
    fn update_membership(&mut self, cfg: EffectiveMembership) -> RaftResult<()> {
        // If the given config does not contain this node's ID, it means one of the following:
        //
        // - the node is currently a non-voter and is replicating an old config to which it has
        // not yet been added.
        // - the node has been removed from the cluster. The parent application can observe the
        // transition to the non-voter state as a signal for when it is safe to shutdown a node
        // being removed.
        self.membership = cfg;
        if !self.membership.membership.contains(&self.id) {
            self.set_target_state(State::NonVoter);
        } else if self.target_state == State::NonVoter && self.membership.membership.members.contains(&self.id) {
            // The node is a NonVoter and the new config has it configured as a normal member.
            // Transition to follower.
            self.set_target_state(State::Follower);
        }
        Ok(())
    }

    /// Update the system's snapshot state based on the given data.
    #[tracing::instrument(level = "trace", skip(self))]
    fn update_snapshot_state(&mut self, update: SnapshotUpdate) {
        if let SnapshotUpdate::SnapshotComplete(log_id) = update {
            self.snapshot_last_log_id = log_id;
            self.report_metrics(Update::Ignore);
        }
        // If snapshot state is anything other than streaming, then drop it.
        if let Some(state @ SnapshotState::Streaming { .. }) = self.snapshot_state.take() {
            self.snapshot_state = Some(state);
        }
    }

    /// Trigger a log compaction (snapshot) job if needed.
    /// If force is True, it will skip the threshold check and start creating snapshot as demanded.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(self) fn trigger_log_compaction_if_needed(&mut self, force: bool) {
        if self.snapshot_state.is_some() {
            return;
        }
        let SnapshotPolicy::LogsSinceLast(threshold) = &self.config.snapshot_policy;
        // Check to ensure we have actual entries for compaction.
        if self.last_applied.index == 0 || self.last_applied.index < self.snapshot_last_log_id.index {
            return;
        }

        if !force {
            // If we are below the threshold, then there is nothing to do.
            if self.last_applied.index < self.snapshot_last_log_id.index + *threshold {
                return;
            }
        }

        // At this point, we are clear to begin a new compaction process.
        let storage = self.storage.clone();
        let (handle, reg) = AbortHandle::new_pair();
        let (chan_tx, _) = broadcast::channel(1);
        let tx_compaction = self.tx_compaction.clone();
        self.snapshot_state = Some(SnapshotState::Snapshotting {
            handle,
            sender: chan_tx.clone(),
        });

        tokio::spawn(
            async move {
                let f = storage.do_log_compaction();
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
    fn reject_init_with_config(&self, tx: oneshot::Sender<Result<(), InitializeError>>) {
        let _ = tx.send(Err(InitializeError::NotAllowed));
    }

    /// Reject a proposed config change request due to the Raft node being in a state which prohibits the request.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    fn reject_config_change_not_leader<T, E>(&self, tx: RaftRespTx<T, E>)
    where E: From<ForwardToLeader> {
        let err = ForwardToLeader {
            leader_id: self.current_leader,
        };

        let _ = tx.send(Err(err.into()));
    }

    /// Forward the given client write request to the leader.
    #[tracing::instrument(level = "trace", skip(self, req, tx))]
    fn forward_client_write_request(
        &self,
        req: ClientWriteRequest<D>,
        tx: RaftRespTx<ClientWriteResponse<R>, ClientWriteError>,
    ) {
        match req.entry {
            EntryPayload::Normal(_entry) => {
                let _ = tx.send(Err(ClientWriteError::ForwardToLeader(ForwardToLeader {
                    leader_id: self.current_leader,
                })));
            }
            _ => {
                // This is unreachable, and well controlled by the type system, but let's log an
                // error for good measure.
                tracing::error!(
                    "unreachable branch hit within async-raft, attempting to forward a Raft internal entry"
                );
            }
        }
    }

    /// Forward the given client read request to the leader.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    fn forward_client_read_request(&self, tx: RaftRespTx<(), ClientReadError>) {
        let _ = tx.send(Err(ClientReadError::ForwardToLeader(ForwardToLeader {
            leader_id: self.current_leader,
        })));
    }
}

#[tracing::instrument(level = "debug", skip(sto), fields(entries=%entries.summary()))]
async fn apply_to_state_machine<D, R, S>(
    sto: Arc<S>,
    entries: &[&Entry<D>],
    max_keep: u64,
) -> Result<Vec<R>, StorageError>
where
    D: AppData,
    R: AppDataResponse,
    S: RaftStorage<D, R>,
{
    let last = entries.last().map(|x| x.log_id);

    if let Some(last_applied) = last {
        // TODO(xp): apply_to_state_machine should return the last applied
        let res = sto.apply_to_state_machine(entries).await?;
        delete_applied_logs(sto, &last_applied, max_keep).await?;
        Ok(res)
    } else {
        Ok(vec![])
    }
}

#[tracing::instrument(level = "debug", skip(sto))]
async fn delete_applied_logs<D, R, S>(sto: Arc<S>, last_applied: &LogId, max_keep: u64) -> Result<(), StorageError>
where
    D: AppData,
    R: AppDataResponse,
    S: RaftStorage<D, R>,
{
    // TODO(xp): periodically batch delete
    let x = last_applied.index + 1;
    let x = x.saturating_sub(max_keep);
    if x > 0 {
        sto.delete_logs_from(..x).await
    } else {
        Ok(())
    }
}

/// An enum describing the way the current leader property is to be updated.
#[derive(Debug)]
pub(self) enum UpdateCurrentLeader {
    Unknown,
    OtherNode(NodeId),
    ThisNode,
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
pub(self) enum SnapshotUpdate {
    /// Snapshot creation has finished successfully and covers the given index.
    SnapshotComplete(LogId),
    /// Snapshot creation failed.
    SnapshotFailed,
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/// All possible states of a Raft node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    /// The node is completely passive; replicating entries, but neither voting nor timing out.
    NonVoter,
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
    /// Check if currently in non-voter state.
    pub fn is_non_voter(&self) -> bool {
        matches!(self, Self::NonVoter)
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
struct LeaderState<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    pub(super) core: &'a mut RaftCore<D, R, N, S>,

    /// A mapping of node IDs the replication state of the target node.
    pub(super) nodes: BTreeMap<NodeId, ReplicationState<D>>,

    /// The metrics about a leader
    pub leader_metrics: LeaderMetrics,

    /// The stream of events coming from replication streams.
    pub(super) replication_rx: mpsc::UnboundedReceiver<(ReplicaEvent<S::SnapshotData>, Span)>,

    /// The cloneable sender channel for replication stream events.
    pub(super) replication_tx: mpsc::UnboundedSender<(ReplicaEvent<S::SnapshotData>, Span)>,

    /// A buffer of client requests which have been appended locally and are awaiting to be committed to the cluster.
    pub(super) awaiting_committed: Vec<ClientRequestEntry<D, R>>,
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> LeaderState<'a, D, R, N, S> {
    /// Create a new instance.
    pub(self) fn new(core: &'a mut RaftCore<D, R, N, S>) -> Self {
        let (replication_tx, replication_rx) = mpsc::unbounded_channel();
        Self {
            core,
            nodes: BTreeMap::new(),
            leader_metrics: LeaderMetrics::default(),
            replication_tx,
            replication_rx,
            awaiting_committed: Vec::new(),
        }
    }

    /// Transition to the Raft leader state.
    #[tracing::instrument(level="trace", skip(self), fields(id=self.core.id, raft_state="leader"))]
    pub(self) async fn run(mut self) -> RaftResult<()> {
        // Spawn replication streams.
        let targets = self
            .core
            .membership
            .membership
            .all_nodes()
            .into_iter()
            .filter(|elem| elem != &self.core.id)
            .collect::<Vec<_>>();

        for target in targets {
            let state = self.spawn_replication_stream(target, None);
            self.nodes.insert(target, state);
        }

        // Setup state as leader.
        self.core.last_heartbeat = None;
        self.core.next_election_timeout = None;
        self.core.update_current_leader(UpdateCurrentLeader::ThisNode);
        self.leader_report_metrics();

        // Per §8, commit an initial entry as part of becoming the cluster leader.
        self.commit_initial_leader_entry().await?;

        loop {
            if !self.core.target_state.is_leader() {
                tracing::info!("id={} state becomes: {:?}", self.core.id, self.core.target_state);

                for node in self.nodes.values() {
                    let _ = node.repl_stream.repl_tx.send((RaftEvent::Terminate, tracing::debug_span!("CH")));
                }
                return Ok(());
            }

            let span = tracing::debug_span!("CHrx:LeaderState");
            let _ent = span.enter();

            tokio::select! {
                Some((msg,span)) = self.core.rx_api.recv() => {
                    let _ent = span.enter();
                    match msg {
                        RaftMsg::AppendEntries{rpc, tx} => {
                            tracing::info!("leader recv from rx_api: AppendEntries, {}", rpc.summary());
                            let _ = tx.send(self.core.handle_append_entries_request(rpc).await);
                        }
                        RaftMsg::RequestVote{rpc, tx} => {
                            tracing::info!("leader recv from rx_api: RequestVote, {}", rpc.summary());
                            let _ = tx.send(self.core.handle_vote_request(rpc).await);
                        }
                        RaftMsg::InstallSnapshot{rpc, tx} => {
                            tracing::info!("leader recv from rx_api: InstallSnapshot, {}", rpc.summary());
                            let _ = tx.send(self.core.handle_install_snapshot_request(rpc).await);
                        }
                        RaftMsg::ClientReadRequest{tx} => {
                            tracing::info!("leader recv from rx_api: ClientReadRequest");
                            self.handle_client_read_request(tx).await;
                        }
                        RaftMsg::ClientWriteRequest{rpc, tx} => {
                            tracing::info!("leader recv from rx_api: ClientWriteRequest, {}", rpc.summary());
                            self.handle_client_write_request(rpc, tx).await;
                        }
                        RaftMsg::Initialize{tx, ..} => {
                            tracing::info!("leader recv from rx_api: Initialize");
                            self.core.reject_init_with_config(tx);
                        }
                        RaftMsg::AddNonVoter{id, tx, blocking} => {
                            tracing::info!("leader recv from rx_api: AddNonVoter, {}", id);
                            self.add_non_voter(id, tx, blocking);
                        }
                        RaftMsg::ChangeMembership{members, blocking, tx} => {
                            tracing::info!("leader recv from rx_api: ChangeMembership, {:?}", members);
                            self.change_membership(members, blocking, tx).await;
                        }
                    }
                },
                Some(update) = self.core.rx_compaction.recv() => {
                    tracing::info!("leader recv from rx_compaction: {:?}", update);
                    self.core.update_snapshot_state(update);
                }
                Some((event, span)) = self.replication_rx.recv() => {
                    tracing::info!("leader recv from replication_rx: {:?}", event.summary());
                    let _ent = span.enter();
                    self.handle_replica_event(event).await;
                }
                Ok(_) = &mut self.core.rx_shutdown => {
                    tracing::info!("leader recv from rx_shudown");
                    self.core.set_target_state(State::Shutdown);
                }
            }
        }
    }

    /// Report metrics with leader specific states.
    #[tracing::instrument(level = "debug", skip(self))]
    pub fn leader_report_metrics(&mut self) {
        self.core.report_metrics(Update::Update(Some(&self.leader_metrics)));
    }
}

/// A struct tracking the state of a replication stream from the perspective of the Raft actor.
struct ReplicationState<D: AppData> {
    pub matched: LogId,
    pub remove_after_commit: Option<u64>,
    pub repl_stream: ReplicationStream<D>,

    /// The response channel to use for when this node has successfully synced with the cluster.
    pub tx: Option<RaftRespTx<AddNonVoterResponse, AddNonVoterError>>,
}

impl<D: AppData> MessageSummary for ReplicationState<D> {
    fn summary(&self) -> String {
        format!(
            "matched: {}, remove_after_commit: {:?}",
            self.matched, self.remove_after_commit
        )
    }
}

impl<D> ReplicationState<D>
where D: AppData
{
    // TODO(xp): make this a method of Config?

    /// Return true if the distance behind last_log_id is smaller than the threshold to join.
    pub fn is_line_rate(&self, last_log_id: &LogId, config: &Config) -> bool {
        is_matched_upto_date(&self.matched, last_log_id, config)
    }
}

pub fn is_matched_upto_date(matched: &LogId, last_log_id: &LogId, config: &Config) -> bool {
    let my_index = matched.index;
    let distance = last_log_id.index.saturating_sub(my_index);
    distance <= config.replication_lag_threshold
}

/// Volatile state specific to a Raft node in candidate state.
struct CandidateState<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    core: &'a mut RaftCore<D, R, N, S>,
    /// The number of votes which have been granted by peer nodes of the old (current) config group.
    votes_granted_old: u64,
    /// The number of votes needed from the old (current) config group in order to become the Raft leader.
    votes_needed_old: u64,
    /// The number of votes which have been granted by peer nodes of the new config group (if applicable).
    votes_granted_new: u64,
    /// The number of votes needed from the new config group in order to become the Raft leader (if applicable).
    votes_needed_new: u64,
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> CandidateState<'a, D, R, N, S> {
    pub(self) fn new(core: &'a mut RaftCore<D, R, N, S>) -> Self {
        Self {
            core,
            votes_granted_old: 0,
            votes_needed_old: 0,
            votes_granted_new: 0,
            votes_needed_new: 0,
        }
    }

    /// Run the candidate loop.
    #[tracing::instrument(level="trace", skip(self), fields(id=self.core.id, raft_state="candidate"))]
    pub(self) async fn run(mut self) -> RaftResult<()> {
        // Each iteration of the outer loop represents a new term.
        loop {
            if !self.core.target_state.is_candidate() {
                return Ok(());
            }

            // Setup initial state per term.
            self.votes_granted_old = 1; // We must vote for ourselves per the Raft spec.
            self.votes_needed_old = ((self.core.membership.membership.members.len() / 2) + 1) as u64; // Just need a majority.
            if let Some(nodes) = &self.core.membership.membership.members_after_consensus {
                self.votes_granted_new = 1; // We must vote for ourselves per the Raft spec.
                self.votes_needed_new = ((nodes.len() / 2) + 1) as u64; // Just need a majority.
            }

            // Setup new term.
            self.core.update_next_election_timeout(false); // Generates a new rand value within range.
            self.core.current_term += 1;
            self.core.voted_for = Some(self.core.id);
            self.core.update_current_leader(UpdateCurrentLeader::Unknown);
            self.core.save_hard_state().await?;
            self.core.report_metrics(Update::Update(None));

            // Send RPCs to all members in parallel.
            let mut pending_votes = self.spawn_parallel_vote_requests();

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
                    Some((res, peer)) = pending_votes.recv() => self.handle_vote_response(res, peer).await?,
                    Some((msg,span)) = self.core.rx_api.recv() => {

                        let _ent = span.enter();

                        match msg {
                            RaftMsg::AppendEntries{rpc, tx} => {
                                let _ = tx.send(self.core.handle_append_entries_request(rpc).await);
                            }
                            RaftMsg::RequestVote{rpc, tx} => {
                                let _ = tx.send(self.core.handle_vote_request(rpc).await);
                            }
                            RaftMsg::InstallSnapshot{rpc, tx} => {
                                let _ = tx.send(self.core.handle_install_snapshot_request(rpc).await);
                            }
                            RaftMsg::ClientReadRequest{tx} => {
                                self.core.forward_client_read_request(tx);
                            }
                            RaftMsg::ClientWriteRequest{rpc, tx} => {
                                self.core.forward_client_write_request(rpc, tx);
                            }
                            RaftMsg::Initialize{tx, ..} => {
                                self.core.reject_init_with_config(tx);
                            }
                            RaftMsg::AddNonVoter{tx, ..} => {
                                self.core.reject_config_change_not_leader(tx);
                            }
                            RaftMsg::ChangeMembership{tx, ..} => {
                                self.core.reject_config_change_not_leader(tx);
                            }
                        }
                    },
                    Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),
                    Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(State::Shutdown),
                }
            }
        }
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/// Volatile state specific to a Raft node in follower state.
pub struct FollowerState<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    core: &'a mut RaftCore<D, R, N, S>,
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> FollowerState<'a, D, R, N, S> {
    pub(self) fn new(core: &'a mut RaftCore<D, R, N, S>) -> Self {
        Self { core }
    }

    /// Run the follower loop.
    #[tracing::instrument(level="trace", skip(self), fields(id=self.core.id, raft_state="follower"))]
    pub(self) async fn run(self) -> RaftResult<()> {
        self.core.report_metrics(Update::Update(None));
        loop {
            if !self.core.target_state.is_follower() {
                return Ok(());
            }
            let election_timeout = sleep_until(self.core.get_next_election_timeout()); // Value is updated as heartbeats are received.

            let span = tracing::debug_span!("CHrx:FollowerState");
            let _ent = span.enter();

            tokio::select! {
                // If an election timeout is hit, then we need to transition to candidate.
                _ = election_timeout => self.core.set_target_state(State::Candidate),
                Some((msg,span)) = self.core.rx_api.recv() => {

                    let _ent = span.enter();

                    match msg {
                        RaftMsg::AppendEntries{rpc, tx} => {
                            let _ = tx.send(self.core.handle_append_entries_request(rpc).await);
                        }
                        RaftMsg::RequestVote{rpc, tx} => {
                            let _ = tx.send(self.core.handle_vote_request(rpc).await);
                        }
                        RaftMsg::InstallSnapshot{rpc, tx} => {
                            let _ = tx.send(self.core.handle_install_snapshot_request(rpc).await);
                        }
                        RaftMsg::ClientReadRequest{tx} => {
                            self.core.forward_client_read_request(tx);
                        }
                        RaftMsg::ClientWriteRequest{rpc, tx} => {
                            self.core.forward_client_write_request(rpc, tx);
                        }
                        RaftMsg::Initialize{tx, ..} => {
                            self.core.reject_init_with_config(tx);
                        }
                        RaftMsg::AddNonVoter{tx, ..} => {
                            self.core.reject_config_change_not_leader(tx);
                        }
                        RaftMsg::ChangeMembership{tx, ..} => {
                            self.core.reject_config_change_not_leader(tx);
                        }
                    }
                },
                Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),
                Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(State::Shutdown),
            }
        }
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/// Volatile state specific to a Raft node in non-voter state.
pub struct NonVoterState<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    core: &'a mut RaftCore<D, R, N, S>,
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> NonVoterState<'a, D, R, N, S> {
    pub(self) fn new(core: &'a mut RaftCore<D, R, N, S>) -> Self {
        Self { core }
    }

    /// Run the non-voter loop.
    #[tracing::instrument(level="trace", skip(self), fields(id=self.core.id, raft_state="non-voter"))]
    pub(self) async fn run(mut self) -> RaftResult<()> {
        self.core.report_metrics(Update::Update(None));
        loop {
            if !self.core.target_state.is_non_voter() {
                return Ok(());
            }

            let span = tracing::debug_span!("CHrx:NonVoterState");
            let _ent = span.enter();

            tokio::select! {
                Some((msg,span)) = self.core.rx_api.recv() => {

                    let _ent = span.enter();

                    match msg {
                        RaftMsg::AppendEntries{rpc, tx} => {
                            let _ = tx.send(self.core.handle_append_entries_request(rpc).await);
                        }
                        RaftMsg::RequestVote{rpc, tx} => {
                            let _ = tx.send(self.core.handle_vote_request(rpc).await);
                        }
                        RaftMsg::InstallSnapshot{rpc, tx} => {
                            let _ = tx.send(self.core.handle_install_snapshot_request(rpc).await);
                        }
                        RaftMsg::ClientReadRequest{tx} => {
                            self.core.forward_client_read_request(tx);
                        }
                        RaftMsg::ClientWriteRequest{rpc, tx} => {
                            self.core.forward_client_write_request(rpc, tx);
                        }
                        RaftMsg::Initialize{members, tx} => {
                            let _ = tx.send(self.handle_init_with_config(members).await);
                        }
                        RaftMsg::AddNonVoter{tx, ..} => {
                            self.core.reject_config_change_not_leader(tx);
                        }
                        RaftMsg::ChangeMembership{tx, ..} => {
                            self.core.reject_config_change_not_leader(tx);
                        }
                    }
                },
                Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),
                Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(State::Shutdown),
            }
        }
    }
}
