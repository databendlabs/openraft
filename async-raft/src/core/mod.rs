//! The core logic of a Raft node.

mod admin;
mod append_entries;
mod client;
mod install_snapshot;
pub(crate) mod replication;
mod vote;

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use futures::future::{AbortHandle, Abortable};
use futures::stream::{FuturesOrdered, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{sleep_until, Duration, Instant};
use tracing_futures::Instrument;

use crate::config::{Config, SnapshotPolicy};
use crate::core::client::ClientRequestEntry;
use crate::error::{ChangeConfigError, ClientReadError, ClientWriteError, InitializeError, RaftError, RaftResult};
use crate::metrics::RaftMetrics;
use crate::raft::{
    ChangeMembershipTx, ClientReadResponseTx, ClientWriteRequest, ClientWriteResponseTx, Entry, EntryPayload, MembershipConfig, RaftMsg,
};
use crate::replication::{RaftEvent, ReplicaEvent, ReplicationStream};
use crate::storage::HardState;
use crate::{AppData, AppDataResponse, NodeId, RaftNetwork, RaftStorage};

/// The core type implementing the Raft protocol.
pub struct RaftCore<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    /// This node's ID.
    id: NodeId,
    /// This node's runtime config.
    config: Arc<Config>,
    /// The cluster's current membership configuration.
    membership: MembershipConfig,
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
    /// The index of the highest log entry which has been applied to the local state machine.
    ///
    /// Is initialized to 0 for a pristine node; else, for nodes with existing state it is
    /// is initialized to the value returned from the `RaftStorage::get_initial_state` on startup.
    /// This value increases following the `commit_index` as logs are applied to the state
    /// machine (via the storage interface).
    last_applied: u64,
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

    /// The index of the last entry to be appended to the log.
    last_log_index: u64,
    /// The term of the last entry to be appended to the log.
    last_log_term: u64,

    /// The node's current snapshot state.
    snapshot_state: Option<SnapshotState<S::Snapshot>>,
    /// The index of the current snapshot, if a snapshot exists.
    ///
    /// This is primarily used in making a determination on when a compaction job needs to be triggered.
    snapshot_index: u64,

    /// A cache of entries which are waiting to be replicated to the state machine.
    ///
    /// It is important to note that this cache must only be populated from the AppendEntries RPC
    /// handler, as these values must only ever represent the entries which have been sent from
    /// the current cluster leader.
    ///
    /// Whenever there is a leadership change, this cache will be cleared.
    entries_cache: BTreeMap<u64, Entry<D>>,
    /// The stream of join handles from state machine replication tasks. There will only ever be
    /// a maximum of 1 element at a time.
    ///
    /// This abstraction is needed to ensure that replicating to the state machine does not block
    /// the AppendEntries RPC flow, and to ensure that we have a smooth transition to becoming
    /// leader without concern over duplicate application of entries to the state machine.
    replicate_to_sm_handle: FuturesOrdered<JoinHandle<anyhow::Result<Option<u64>>>>,
    /// A bool indicating if this system has performed its initial replication of
    /// outstanding entries to the state machine.
    has_completed_initial_replication_to_sm: bool,

    /// The last time a heartbeat was received.
    last_heartbeat: Option<Instant>,
    /// The duration until the next election timeout.
    next_election_timeout: Option<Instant>,

    tx_compaction: mpsc::Sender<SnapshotUpdate>,
    rx_compaction: mpsc::Receiver<SnapshotUpdate>,

    rx_api: mpsc::UnboundedReceiver<RaftMsg<D, R>>,
    tx_metrics: watch::Sender<RaftMetrics>,
    rx_shutdown: oneshot::Receiver<()>,
}

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> RaftCore<D, R, N, S> {
    pub(crate) fn spawn(
        id: NodeId, config: Arc<Config>, network: Arc<N>, storage: Arc<S>, rx_api: mpsc::UnboundedReceiver<RaftMsg<D, R>>,
        tx_metrics: watch::Sender<RaftMetrics>, rx_shutdown: oneshot::Receiver<()>,
    ) -> JoinHandle<RaftResult<()>> {
        let membership = MembershipConfig::new_initial(id); // This is updated from storage in the main loop.
        let (tx_compaction, rx_compaction) = mpsc::channel(1);
        let this = Self {
            id,
            config,
            membership,
            network,
            storage,
            target_state: State::Follower,
            commit_index: 0,
            last_applied: 0,
            current_term: 0,
            current_leader: None,
            voted_for: None,
            last_log_index: 0,
            last_log_term: 0,
            snapshot_state: None,
            snapshot_index: 0,
            entries_cache: Default::default(),
            replicate_to_sm_handle: FuturesOrdered::new(),
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
    #[tracing::instrument(level="trace", skip(self), fields(id=self.id, cluster=%self.config.cluster_name))]
    async fn main(mut self) -> RaftResult<()> {
        tracing::trace!("raft node is initializing");
        let state = self.storage.get_initial_state().await.map_err(|err| self.map_fatal_storage_error(err))?;
        self.last_log_index = state.last_log_index;
        self.last_log_term = state.last_log_term;
        self.current_term = state.hard_state.current_term;
        self.voted_for = state.hard_state.voted_for;
        self.membership = state.membership;
        self.last_applied = state.last_applied_log;
        // NOTE: this is repeated here for clarity. It is unsafe to initialize the node's commit
        // index to any other value. The commit index must be determined by a leader after
        // successfully committing a new log to the cluster.
        self.commit_index = 0;

        // Fetch the most recent snapshot in the system.
        if let Some(snapshot) = self
            .storage
            .get_current_snapshot()
            .await
            .map_err(|err| self.map_fatal_storage_error(err))?
        {
            self.snapshot_index = snapshot.index;
        }

        // Set initial state based on state recovered from disk.
        let is_only_configured_member = self.membership.members.len() == 1 && self.membership.contains(&self.id);
        // If this is the only configured member and there is live state, then this is
        // a single-node cluster. Become leader.
        if is_only_configured_member && self.last_log_index != u64::min_value() {
            self.target_state = State::Leader;
        }
        // Else if there are other members, that can only mean that state was recovered. Become follower.
        // Here we use a 30 second overhead on the initial next_election_timeout. This is because we need
        // to ensure that restarted nodes don't disrupt a stable cluster by timing out and driving up their
        // term before network communication is established.
        else if !is_only_configured_member && self.membership.contains(&self.id) {
            self.target_state = State::Follower;
            let inst = Instant::now() + Duration::from_secs(30) + Duration::from_millis(self.config.new_rand_election_timeout());
            self.next_election_timeout = Some(inst);
        }
        // Else, for any other condition, stay non-voter.
        else {
            self.target_state = State::NonVoter;
        }

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
    #[tracing::instrument(level = "trace", skip(self))]
    fn report_metrics(&mut self) {
        let res = self.tx_metrics.send(RaftMetrics {
            id: self.id,
            state: self.target_state,
            current_term: self.current_term,
            last_log_index: self.last_log_index,
            last_applied: self.last_applied,
            current_leader: self.current_leader,
            membership_config: self.membership.clone(),
        });
        if let Err(err) = res {
            tracing::error!({error=%err, id=self.id}, "error reporting metrics");
        }
    }

    /// Save the Raft node's current hard state to disk.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn save_hard_state(&mut self) -> RaftResult<()> {
        let hs = HardState {
            current_term: self.current_term,
            voted_for: self.voted_for,
        };
        Ok(self.storage.save_hard_state(&hs).await.map_err(|err| self.map_fatal_storage_error(err))?)
    }

    /// Update core's target state, ensuring all invariants are upheld.
    #[tracing::instrument(level = "trace", skip(self))]
    fn set_target_state(&mut self, target_state: State) {
        if target_state == State::Follower && !self.membership.contains(&self.id) {
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
                let inst = Instant::now() + Duration::from_millis(self.config.new_rand_election_timeout());
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
        self.next_election_timeout = Some(now + Duration::from_millis(self.config.new_rand_election_timeout()));
        if heartbeat {
            self.last_heartbeat = Some(now);
        }
    }

    /// Update the value of the `current_leader` property.
    #[tracing::instrument(level = "trace", skip(self))]
    fn update_current_leader(&mut self, update: UpdateCurrentLeader) {
        self.entries_cache.clear();
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
        tracing::error!({error=%err, id=self.id}, "fatal storage error, shutting down");
        self.set_target_state(State::Shutdown);
        RaftError::RaftStorage(err)
    }

    /// Update the node's current membership config & save hard state.
    #[tracing::instrument(level = "trace", skip(self))]
    fn update_membership(&mut self, cfg: MembershipConfig) -> RaftResult<()> {
        // If the given config does not contain this node's ID, it means one of the following:
        //
        // - the node is currently a non-voter and is replicating an old config to which it has
        // not yet been added.
        // - the node has been removed from the cluster. The parent application can observe the
        // transition to the non-voter state as a signal for when it is safe to shutdown a node
        // being removed.
        self.membership = cfg;
        if !self.membership.contains(&self.id) {
            self.set_target_state(State::NonVoter);
        } else if self.target_state == State::NonVoter && self.membership.members.contains(&self.id) {
            // The node is a NonVoter and the new config has it configured as a normal member.
            // Transition to follower.
            self.set_target_state(State::Follower);
        }
        Ok(())
    }

    /// Update the system's snapshot state based on the given data.
    #[tracing::instrument(level = "trace", skip(self))]
    fn update_snapshot_state(&mut self, update: SnapshotUpdate) {
        if let SnapshotUpdate::SnapshotComplete(index) = update {
            self.snapshot_index = index
        }
        // If snapshot state is anything other than streaming, then drop it.
        if let Some(state @ SnapshotState::Streaming { .. }) = self.snapshot_state.take() {
            self.snapshot_state = Some(state)
        }
    }

    /// Trigger a log compaction (snapshot) job if needed.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(self) fn trigger_log_compaction_if_needed(&mut self) {
        if self.snapshot_state.is_some() {
            return;
        }
        let SnapshotPolicy::LogsSinceLast(threshold) = &self.config.snapshot_policy;
        // Check to ensure we have actual entries for compaction.
        if self.last_applied == 0 || self.last_applied < self.snapshot_index {
            return;
        }
        // If we are below the threshold, then there is nothing to do.
        let is_below_threshold = self
            .last_applied
            .checked_sub(self.snapshot_index)
            .map(|diff| diff < *threshold)
            .unwrap_or(false);
        if is_below_threshold {
            return;
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
                let res = Abortable::new(storage.do_log_compaction(), reg).await;
                match res {
                    Ok(res) => match res {
                        Ok(snapshot) => {
                            let _ = tx_compaction.try_send(SnapshotUpdate::SnapshotComplete(snapshot.index));
                            let _ = chan_tx.send(snapshot.index); // This will always succeed.
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

    /// Handle the output of an async task replicating entries to the state machine.
    #[tracing::instrument(level = "trace", skip(self, res))]
    pub(self) fn handle_replicate_to_sm_result(&mut self, res: anyhow::Result<Option<u64>>) -> RaftResult<()> {
        let last_applied_opt = res.map_err(|err| self.map_fatal_storage_error(err))?;
        if let Some(last_applied) = last_applied_opt {
            self.last_applied = last_applied;
        }
        self.report_metrics();
        self.trigger_log_compaction_if_needed();
        Ok(())
    }

    /// Reject an init config request due to the Raft node being in a state which prohibits the request.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    fn reject_init_with_config(&self, tx: oneshot::Sender<Result<(), InitializeError>>) {
        let _ = tx.send(Err(InitializeError::NotAllowed));
    }

    /// Reject a proposed config change request due to the Raft node being in a state which prohibits the request.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    fn reject_config_change_not_leader(&self, tx: oneshot::Sender<Result<(), ChangeConfigError>>) {
        let _ = tx.send(Err(ChangeConfigError::NodeNotLeader(self.current_leader)));
    }

    /// Forward the given client write request to the leader.
    #[tracing::instrument(level = "trace", skip(self, req, tx))]
    fn forward_client_write_request(&self, req: ClientWriteRequest<D>, tx: ClientWriteResponseTx<D, R>) {
        match req.entry {
            EntryPayload::Normal(entry) => {
                let _ = tx.send(Err(ClientWriteError::ForwardToLeader(entry.data, self.current_leader)));
            }
            _ => {
                // This is unreachable, and well controlled by the type system, but let's log an
                // error for good measure.
                tracing::error!("unreachable branch hit within async-raft, attempting to forward a Raft internal entry");
            }
        }
    }

    /// Forward the given client read request to the leader.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    fn forward_client_read_request(&self, tx: ClientReadResponseTx) {
        let _ = tx.send(Err(ClientReadError::ForwardToLeader(self.current_leader)));
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
    SnapshotComplete(u64),
    /// Snapshot creation failed.
    SnapshotFailed,
}

///////////////////////////////////////////////////////////////////////////////////////////////////
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
///////////////////////////////////////////////////////////////////////////////////////////////////

/// Volatile state specific to the Raft leader.
struct LeaderState<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> {
    pub(super) core: &'a mut RaftCore<D, R, N, S>,
    /// A mapping of node IDs the replication state of the target node.
    pub(super) nodes: BTreeMap<NodeId, ReplicationState<D>>,
    /// A mapping of new nodes (non-voters) which are being synced in order to join the cluster.
    pub(super) non_voters: BTreeMap<NodeId, NonVoterReplicationState<D>>,
    /// A bool indicating if this node will be stepping down after committing the current config change.
    pub(super) is_stepping_down: bool,

    /// The stream of events coming from replication streams.
    pub(super) replicationrx: mpsc::UnboundedReceiver<ReplicaEvent<S::Snapshot>>,
    /// The clonable sender channel for replication stream events.
    pub(super) replicationtx: mpsc::UnboundedSender<ReplicaEvent<S::Snapshot>>,
    /// A buffer of client requests which have been appended locally and are awaiting to be committed to the cluster.
    pub(super) awaiting_committed: Vec<ClientRequestEntry<D, R>>,
    /// A field tracking the cluster's current consensus state, which is used for dynamic membership.
    pub(super) consensus_state: ConsensusState,

    /// An optional response channel for when a config change has been proposed, and is awaiting a response.
    pub(super) propose_config_change_cb: Option<oneshot::Sender<Result<(), RaftError>>>,
    /// An optional receiver for when a joint consensus config is committed.
    pub(super) joint_consensus_cb: FuturesOrdered<oneshot::Receiver<Result<u64, RaftError>>>,
    /// An optional receiver for when a uniform consensus config is committed.
    pub(super) uniform_consensus_cb: FuturesOrdered<oneshot::Receiver<Result<u64, RaftError>>>,
}

impl<'a, D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> LeaderState<'a, D, R, N, S> {
    /// Create a new instance.
    pub(self) fn new(core: &'a mut RaftCore<D, R, N, S>) -> Self {
        let consensus_state = if core.membership.is_in_joint_consensus() {
            ConsensusState::Joint { is_committed: false }
        } else {
            ConsensusState::Uniform
        };
        let (replicationtx, replicationrx) = mpsc::unbounded_channel();
        Self {
            core,
            nodes: BTreeMap::new(),
            non_voters: BTreeMap::new(),
            is_stepping_down: false,
            replicationtx,
            replicationrx,
            consensus_state,
            awaiting_committed: Vec::new(),
            propose_config_change_cb: None,
            joint_consensus_cb: FuturesOrdered::new(),
            uniform_consensus_cb: FuturesOrdered::new(),
        }
    }

    /// Transition to the Raft leader state.
    #[tracing::instrument(level="trace", skip(self), fields(id=self.core.id, raft_state="leader"))]
    pub(self) async fn run(mut self) -> RaftResult<()> {
        // Spawn replication streams.
        let targets = self
            .core
            .membership
            .all_nodes()
            .into_iter()
            .filter(|elem| elem != &self.core.id)
            .collect::<Vec<_>>();
        for target in targets {
            let state = self.spawn_replication_stream(target);
            self.nodes.insert(target, state);
        }

        // Setup state as leader.
        self.core.last_heartbeat = None;
        self.core.next_election_timeout = None;
        self.core.update_current_leader(UpdateCurrentLeader::ThisNode);
        self.core.report_metrics();

        // Per §8, commit an initial entry as part of becoming the cluster leader.
        self.commit_initial_leader_entry().await?;

        loop {
            if !self.core.target_state.is_leader() {
                for node in self.nodes.values() {
                    let _ = node.replstream.repltx.send(RaftEvent::Terminate);
                }
                for node in self.non_voters.values() {
                    let _ = node.state.replstream.repltx.send(RaftEvent::Terminate);
                }
                return Ok(());
            }
            tokio::select! {
                Some(msg) = self.core.rx_api.recv() => match msg {
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
                        self.handle_client_read_request(tx).await;
                    }
                    RaftMsg::ClientWriteRequest{rpc, tx} => {
                        self.handle_client_write_request(rpc, tx).await;
                    }
                    RaftMsg::Initialize{tx, ..} => {
                        self.core.reject_init_with_config(tx);
                    }
                    RaftMsg::AddNonVoter{id, tx} => {
                        self.add_member(id, tx);
                    }
                    RaftMsg::ChangeMembership{members, tx} => {
                        self.change_membership(members, tx).await;
                    }
                },
                Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),
                Some(Ok(res)) = self.joint_consensus_cb.next() => {
                    match res {
                        Ok(_) => self.handle_joint_consensus_committed().await?,
                        Err(err) => if let Some(cb) = self.propose_config_change_cb.take() {
                            let _ = cb.send(Err(err));
                        }
                    }
                }
                Some(Ok(res)) = self.uniform_consensus_cb.next() => {
                    match res {
                        Ok(index) => {
                            let final_res = self.handle_uniform_consensus_committed(index).await;
                            if let Some(cb) = self.propose_config_change_cb.take() {
                                let _ = cb.send(final_res.map_err(From::from));
                            }
                        }
                        Err(err) => if let Some(cb) = self.propose_config_change_cb.take() {
                            let _ = cb.send(Err(err));
                        }
                    }
                }
                Some(event) = self.replicationrx.recv() => self.handle_replica_event(event).await,
                Some(Ok(repl_sm_result)) = self.core.replicate_to_sm_handle.next() => {
                    // Errors herein will trigger shutdown, so no need to process error.
                    let _ = self.core.handle_replicate_to_sm_result(repl_sm_result);
                }
                Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(State::Shutdown),
            }
        }
    }
}

/// A struct tracking the state of a replication stream from the perspective of the Raft actor.
struct ReplicationState<D: AppData> {
    pub match_index: u64,
    pub match_term: u64,
    pub is_at_line_rate: bool,
    pub remove_after_commit: Option<u64>,
    pub replstream: ReplicationStream<D>,
}

/// The same as `ReplicationState`, except for non-voters.
struct NonVoterReplicationState<D: AppData> {
    /// The replication stream state.
    pub state: ReplicationState<D>,
    /// A bool indicating if this non-voters is ready to join the cluster.
    pub is_ready_to_join: bool,
    /// The response channel to use for when this node has successfully synced with the cluster.
    pub tx: Option<oneshot::Sender<Result<(), ChangeConfigError>>>,
}

/// A state enum used by Raft leaders to navigate the joint consensus protocol.
pub enum ConsensusState {
    /// The cluster is preparring to go into joint consensus, but the leader is still syncing
    /// some non-voters to prepare them for cluster membership.
    NonVoterSync {
        /// The set of non-voters nodes which are still being synced.
        awaiting: HashSet<NodeId>,
        /// The full membership change which has been proposed.
        members: HashSet<NodeId>,
        /// The response channel to use once the consensus state is back into uniform state.
        tx: ChangeMembershipTx,
    },
    /// The cluster is in a joint consensus state and is syncing new nodes.
    Joint {
        /// A bool indicating if the associated config which started this joint consensus has yet been comitted.
        ///
        /// NOTE: when a new leader is elected, it will initialize this value to false, and then
        /// update this value to true once the new leader's blank payload has been committed.
        is_committed: bool,
    },
    /// The cluster consensus is uniform; not in a joint consensus state.
    Uniform,
}

impl ConsensusState {
    /// Check the current state to determine if it is in joint consensus, and if it is safe to finalize the joint consensus.
    ///
    /// The return value will be true if:
    /// 1. this object currently represents a joint consensus state.
    /// 2. the corresponding config for this consensus state has been committed to the cluster.
    pub fn is_joint_consensus_safe_to_finalize(&self) -> bool {
        match self {
            ConsensusState::Joint { is_committed } => *is_committed,
            _ => false,
        }
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////////////////////////

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
            self.votes_needed_old = ((self.core.membership.members.len() / 2) + 1) as u64; // Just need a majority.
            if let Some(nodes) = &self.core.membership.members_after_consensus {
                self.votes_granted_new = 1; // We must vote for ourselves per the Raft spec.
                self.votes_needed_new = ((nodes.len() / 2) + 1) as u64; // Just need a majority.
            }

            // Setup new term.
            self.core.update_next_election_timeout(false); // Generates a new rand value within range.
            self.core.current_term += 1;
            self.core.voted_for = Some(self.core.id);
            self.core.update_current_leader(UpdateCurrentLeader::Unknown);
            self.core.save_hard_state().await?;
            self.core.report_metrics();

            // Send RPCs to all members in parallel.
            let mut pending_votes = self.spawn_parallel_vote_requests();

            // Inner processing loop for this Raft state.
            loop {
                if !self.core.target_state.is_candidate() {
                    return Ok(());
                }
                let timeout_fut = sleep_until(self.core.get_next_election_timeout());
                tokio::select! {
                    _ = timeout_fut => break, // This election has timed-out. Break to outer loop, which starts a new term.
                    Some((res, peer)) = pending_votes.recv() => self.handle_vote_response(res, peer).await?,
                    Some(msg) = self.core.rx_api.recv() => match msg {
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
                    },
                    Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),
                    Some(Ok(repl_sm_result)) = self.core.replicate_to_sm_handle.next() => {
                        // Errors herein will trigger shutdown, so no need to process error.
                        let _ = self.core.handle_replicate_to_sm_result(repl_sm_result);
                    }
                    Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(State::Shutdown),
                }
            }
        }
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////
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
        self.core.report_metrics();
        loop {
            if !self.core.target_state.is_follower() {
                return Ok(());
            }

            let election_timeout = sleep_until(self.core.get_next_election_timeout()); // Value is updated as heartbeats are received.
            tokio::select! {
                // If an election timeout is hit, then we need to transition to candidate.
                _ = election_timeout => self.core.set_target_state(State::Candidate),
                Some(msg) = self.core.rx_api.recv() => match msg {
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
                },
                Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),
                Some(Ok(repl_sm_result)) = self.core.replicate_to_sm_handle.next() => {
                    // Errors herein will trigger shutdown, so no need to process error.
                    let _ = self.core.handle_replicate_to_sm_result(repl_sm_result);
                }
                Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(State::Shutdown),
            }
        }
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////
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
        self.core.report_metrics();
        loop {
            if !self.core.target_state.is_non_voter() {
                return Ok(());
            }
            tokio::select! {
                Some(msg) = self.core.rx_api.recv() => match msg {
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
                },
                Some(update) = self.core.rx_compaction.recv() => self.core.update_snapshot_state(update),
                Some(Ok(repl_sm_result)) = self.core.replicate_to_sm_handle.next() => {
                    // Errors herein will trigger shutdown, so no need to process error.
                    let _ = self.core.handle_replicate_to_sm_result(repl_sm_result);
                }
                Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(State::Shutdown),
            }
        }
    }
}
