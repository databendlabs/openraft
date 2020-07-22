//! The core logic of a Raft node.

mod admin;
mod append_entries;
mod apply_entries;
mod client;
mod install_snapshot;
pub(crate) mod replication;
mod vote;

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::{Abortable, AbortHandle};
use futures::stream::FuturesOrdered;
use tokio::stream::StreamExt;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, Duration, delay_until};
use tracing_futures::Instrument;

use crate::{AppData, AppDataResponse, AppError, RaftNetwork, RaftStorage, NodeId};
use crate::error::{ClientError, InitWithConfigError, ProposeConfigChangeError, RaftResult};
use crate::config::{Config, SnapshotPolicy};
use crate::raft::MembershipConfig;
use crate::metrics::{RaftMetrics, State};
use crate::core::client::ClientRequestEntry;
use crate::raft::{RxChanAppendEntries, RxChanVote, RxChanInstallSnapshot, RxChanClient, RxChanInit, RxChanPropose};
use crate::raft::{ClientRequest, ClientResponse, InitWithConfig, ProposeConfigChange};
use crate::raft::{TxClientResponse, TxInitResponse, TxProposeResponse};
use crate::replication::{RaftEvent, ReplicationStream, ReplicaEvent};
use crate::storage::{HardState, SnapshotWriter};

/// The core type implementing the Raft protocol.
pub struct RaftCore<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> {
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
    target_state: TargetState,

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
    /// Is initialized to 0, increases following the `commit_index` as logs are
    /// applied to the state machine (via the storage interface).
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
    snapshot_state: Option<SnapshotState>,
    /// The index of the current snapshot, if a snapshot exists.
    ///
    /// This is primarily used in making a determination on when a compaction job needs to be triggered.
    snapshot_index: u64,

    /// The last time a heartbeat was received.
    last_heartbeat: Option<Instant>,
    /// The duration until the next election timeout.
    next_election_timeout: Option<Instant>,

    tx_compaction: mpsc::Sender<SnapshotUpdate>,
    rx_compaction: mpsc::Receiver<SnapshotUpdate>,

    rx_append_entries: RxChanAppendEntries<D, E>,
    rx_vote: RxChanVote<E>,
    rx_install_snapshot: RxChanInstallSnapshot<E>,
    rx_client: RxChanClient<D, R, E>,
    rx_init: RxChanInit<E>,
    rx_propose: RxChanPropose<E>,
    tx_metrics: watch::Sender<RaftMetrics>,
}

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> RaftCore<D, R, E, N, S> {
    pub fn spawn(
        id: NodeId, config: Config, network: Arc<N>, storage: Arc<S>,
        rx_append_entries: RxChanAppendEntries<D, E>,
        rx_vote: RxChanVote<E>,
        rx_install_snapshot: RxChanInstallSnapshot<E>,
        rx_client: RxChanClient<D, R, E>,
        rx_init: RxChanInit<E>,
        rx_propose: RxChanPropose<E>,
        tx_metrics: watch::Sender<RaftMetrics>,
    ) -> JoinHandle<RaftResult<(), E>> {
        let config = Arc::new(config);
        let membership = MembershipConfig{is_in_joint_consensus: false, members: vec![id], non_voters: vec![], removing: vec![]};
        let (tx_compaction, rx_compaction) = mpsc::channel(1);
        let this = Self{
            id, config, membership, network, storage,
            target_state: TargetState::Follower,
            commit_index: 0, last_applied: 0, current_term: 0, current_leader: None, voted_for: None,
            last_log_index: 0, last_log_term: 0,
            snapshot_state: None, snapshot_index: 0,
            last_heartbeat: None, next_election_timeout: None,
            tx_compaction, rx_compaction,
            rx_append_entries, rx_vote, rx_install_snapshot, rx_client,
            rx_init, rx_propose, tx_metrics,
        };
        tokio::spawn(this.main())
    }

    /// The main loop of the Raft protocol.
    #[tracing::instrument(level="debug", skip(self), fields(id=self.id))]
    async fn main(mut self) -> RaftResult<(), E> {
        let state = self.storage.get_initial_state().await?;
        self.last_log_index = state.last_log_index;
        self.last_log_term = state.last_log_term;
        self.current_term = state.hard_state.current_term;
        self.voted_for = state.hard_state.voted_for;
        self.membership = state.hard_state.membership;
        self.last_applied = state.last_applied_log;
        // NOTE: this is repeated here for clarity. It is unsafe to initialize the node's commit
        // index to any other value. The commit index must be determined by a leader after
        // successfully committing a new log to the cluster.
        self.commit_index = 0;

        // Fetch the most recent snapshot in the system.
        if let Some(snapshot) = self.storage.get_current_snapshot().await? {
            self.snapshot_index = snapshot.index;
        }

        // Set initial state based on state recovered from disk.
        let is_only_configured_member = self.membership.len() == 1 && self.membership.contains(&self.id);
        // If this is the only configured member and there is live state, then this is
        // a single-node cluster. Become leader.
        if is_only_configured_member && &self.last_log_index != &u64::min_value() {
            self.target_state = TargetState::Leader;
        }
        // Else if there are other members, that can only mean that state was recovered. Become follower.
        else if !is_only_configured_member {
            self.target_state = TargetState::Follower;
        }
        // Else, for any other condition, stay non-voter.
        else {
            self.target_state = TargetState::NonVoter;
        }

        // This is central loop of the system. The Raft core assumes a few different roles based
        // on cluster state. The Raft core will delegate control to the different state
        // controllers and simply awaits the delegated loop to return, which will only take place
        // if some error has been encountered, or if a state change is required.
        loop {
            match &self.target_state {
                TargetState::Leader => LeaderState::new(&mut self).run().await?,
                TargetState::Candidate => CandidateState::new(&mut self).run().await?,
                TargetState::Follower => FollowerState::new(&mut self).run().await?,
                TargetState::NonVoter => NonVoterState::new(&mut self).run().await?,
                TargetState::Shutdown => return Ok(()),
            }
        }
    }

    /// Report a metrics payload on the current state of the Raft node.
    #[tracing::instrument(level="debug", skip(self, state))]
    fn report_metrics(&mut self, state: State) {
        let res = self.tx_metrics.broadcast(RaftMetrics{
            id: self.id, state, current_term: self.current_term,
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
    #[tracing::instrument(level="debug", skip(self))]
    async fn save_hard_state(&mut self) -> RaftResult<(), E> {
        let hs = HardState{current_term: self.current_term, voted_for: self.voted_for, membership: self.membership.clone()};
        Ok(self.storage.save_hard_state(&hs).await.map_err(|err| self.map_fatal_storage_result(err))?)
    }

    /// Update core's target state, ensuring all invariants are upheld.
    #[tracing::instrument(level="debug", skip(self))]
    fn set_target_state(&mut self, target_state: TargetState) {
        if &target_state == &TargetState::Follower && self.membership.non_voters.contains(&self.id) {
            self.target_state = TargetState::NonVoter;
        }
        self.target_state = target_state;
    }

    /// Get the next election timeout, generating a new value if not set.
    #[tracing::instrument(level="trace", skip(self))]
    fn get_next_election_timeout(&mut self) -> Instant {
        match self.next_election_timeout {
            Some(inst) => inst.clone(),
            None => {
                let inst = Instant::now() + Duration::from_millis(self.config.new_rand_election_timeout());
                self.next_election_timeout = Some(inst.clone());
                inst
            }
        }
    }

    /// Set a value for the next election timeout.
    #[tracing::instrument(level="trace", skip(self))]
    fn update_next_election_timeout(&mut self) {
        self.next_election_timeout = Some(Instant::now() + Duration::from_millis(self.config.new_rand_election_timeout()));
    }

    /// Update the value of the `current_leader` property.
    #[tracing::instrument(level="debug", skip(self))]
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
            },
        }
    }

    /// Encapsulate the process of updating the current term, as updating the `voted_for` state must also be updated.
    #[tracing::instrument(level="debug", skip(self))]
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
    #[tracing::instrument(level="debug", skip(self))]
    fn map_fatal_storage_result<Err: std::error::Error>(&mut self, err: Err) -> Err {
        tracing::error!({error=%err, id=self.id}, "fatal storage error, shutting down");
        self.set_target_state(TargetState::Shutdown);
        err
    }

    /// Update the node's current membership config & save hard state.
    #[tracing::instrument(level="debug", skip(self))]
    async fn update_membership(&mut self, cfg: MembershipConfig) -> RaftResult<(), E> {
        // If the given config does not contain this node's ID, it means one of the following:
        //
        // - the node is currently a non-voter and is replicating an old config to which it has
        // not yet been added.
        // - the node has been removed from the cluster. The parent application can observe the
        // transition to the non-voter state as a signal for when it is safe to shutdown a node
        // being removed.
        self.membership = cfg;
        if !self.membership.contains(&self.id) {
            self.set_target_state(TargetState::NonVoter);
        } else if &self.target_state == &TargetState::NonVoter && self.membership.members.contains(&self.id) {
            // The node is a NonVoter and the new config has it configured as a normal member.
            // Transition to follower.
            self.set_target_state(TargetState::Follower);
        }
        Ok(self.save_hard_state().await?)
    }

    /// Update the system's snapshot state based on the given data.
    #[tracing::instrument(level="debug", skip(self))]
    fn update_snapshot_state(&mut self, update: SnapshotUpdate) {
        if let SnapshotUpdate::SnapshotComplete(index) = update {
            self.snapshot_index = index
        }
        // If snapshot state is anything other than streaming, then drop it.
        match self.snapshot_state.take() {
            Some(state @ SnapshotState::Streaming{..}) => self.snapshot_state = Some(state),
            _ => (),
        }
    }

    /// Trigger a log compaction (snapshot) job if needed.
    #[tracing::instrument(level="debug", skip(self))]
    pub(self) fn trigger_log_compaction_if_needed(&mut self) {
        if self.snapshot_state.is_some() {
            return;
        }
        let threshold = match &self.config.snapshot_policy {
            SnapshotPolicy::LogsSinceLast(threshold) => threshold,
        };
        if &(&self.commit_index - &self.snapshot_index) < threshold {
            return;
        }

        // At this point, we are clear to begin a new compaction process.
        let storage = self.storage.clone();
        let through_index = self.commit_index;
        let (handle, reg) = AbortHandle::new_pair();
        let (chan_tx, _) = broadcast::channel(1);
        let mut tx_compaction = self.tx_compaction.clone();
        self.snapshot_state = Some(SnapshotState::Snapshotting{through: through_index, handle, sender: chan_tx.clone()});
        tokio::spawn(async move {
            let res = Abortable::new(storage.create_snapshot(through_index), reg).await;
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
        }.instrument(tracing::debug_span!("beginning new log compaction process")));
    }

    /// Reject an init config request due to the Raft node being in a state which prohibits the request.
    #[tracing::instrument(level="trace", skip(self, tx))]
    fn reject_init_with_config(&self, _: InitWithConfig, tx: TxInitResponse<E>) {
        let _ = tx.send(Err(InitWithConfigError::NotAllowed));
    }

    /// Reject a proposed config change request due to the Raft node being in a state which prohibits the request.
    #[tracing::instrument(level="trace", skip(self, tx))]
    fn reject_propose_config_change(&self, _: ProposeConfigChange, tx: TxProposeResponse<E>) {
        let _ = tx.send(Err(ProposeConfigChangeError::NodeNotLeader));
    }

    /// Forward the given client request to the leader.
    #[tracing::instrument(level="trace", skip(self, req, tx))]
    fn forward_client_request(&self, req: ClientRequest<D>, tx: TxClientResponse<D, R, E>) {
        let _ = tx.send(Err(ClientError::ForwardToLeader(req, self.voted_for.clone())));
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
pub(self) enum SnapshotState {
    /// The Raft node is compacting itself.
    Snapshotting {
        /// The last included index of the new snapshot being generated.
        through: u64,
        /// A handle to abort the compaction process early if needed.
        handle: AbortHandle,
        /// A sender for notifiying any other tasks of the completion of this compaction.
        sender: broadcast::Sender<u64>,
    },
    /// The Raft node is streaming in a snapshot from the leader.
    Streaming {
        /// The offset of the last byte written to the snapshot.
        offset: u64,
        /// A handle to the snapshot writer.
        snapshot: Box<dyn SnapshotWriter>,
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

/// The desired target state of a Raft node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetState {
    NonVoter,
    Follower,
    Candidate,
    Leader,
    Shutdown,
}

impl TargetState {
    /// Check if currently in non-voter state.
    pub fn is_non_voter(&self) -> bool {
        if let Self::NonVoter = self { true } else { false }
    }

    /// Check if currently in follower state.
    pub fn is_follower(&self) -> bool {
        if let Self::Follower = self { true } else { false }
    }

    /// Check if currently in candidate state.
    pub fn is_candidate(&self) -> bool {
        if let Self::Candidate = self { true } else { false }
    }

    /// Check if currently in leader state.
    pub fn is_leader(&self) -> bool {
        if let Self::Leader = self { true } else { false }
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////////////////////////

/// Volatile state specific to the Raft leader.
///
/// This state is reinitialized after an election.
struct LeaderState<'a, D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> {
    pub(super) core: &'a mut RaftCore<D, R, E, N, S>,
    /// A mapping of node IDs the replication state of the target node.
    pub(super) nodes: BTreeMap<NodeId, ReplicationState<D>>,
    /// The stream of events coming from replication streams.
    pub(super) replicationrx: mpsc::UnboundedReceiver<ReplicaEvent>,
    /// The clonable sender channel for replication stream events.
    pub(super) replicationtx: mpsc::UnboundedSender<ReplicaEvent>,
    /// A buffer of client requests which have been appended locally and are awaiting to be committed to the cluster.
    pub(super) awaiting_committed: Vec<ClientRequestEntry<D, R, E>>,
    /// A field tracking the cluster's current consensus state, which is used for dynamic membership.
    pub(super) consensus_state: ConsensusState,
    /// An optional response channel for when a config change has been proposed, and is awaiting a response.
    pub(super) propose_config_change_cb: Option<oneshot::Sender<Result<(), ClientError<D, E>>>>,
    /// An optional receiver for when a joint consensus config is committed.
    pub(super) joint_consensus_cb: FuturesOrdered<oneshot::Receiver<Result<ClientResponse<R>, ClientError<D, E>>>>,
    /// An optional receiver for when a uniform consensus config is committed.
    pub(super) uniform_consensus_cb: FuturesOrdered<oneshot::Receiver<Result<ClientResponse<R>, ClientError<D, E>>>>,
}

impl<'a, D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> LeaderState<'a, D, R, E, N, S> {
    /// Create a new instance.
    pub(self) fn new(core: &'a mut RaftCore<D, R, E, N, S>) -> Self {
        let consensus_state = if core.membership.is_in_joint_consensus {
            ConsensusState::Joint{
                new_nodes_being_synced: core.membership.non_voters.clone(),
                is_committed: false,
            }
        } else {
            ConsensusState::Uniform
        };
        let (replicationtx, replicationrx) = mpsc::unbounded_channel();
        Self{
            core, nodes: BTreeMap::new(), awaiting_committed: Vec::new(), consensus_state,
            propose_config_change_cb: None, joint_consensus_cb: FuturesOrdered::new(), uniform_consensus_cb: FuturesOrdered::new(),
            replicationtx, replicationrx,
        }
    }

    /// Transition to the Raft leader state.
    #[tracing::instrument(level="debug", skip(self), fields(id=self.core.id, raft_state="leader"))]
    pub(self) async fn run(mut self) -> RaftResult<(), E> {
        // Spawn replication streams.
        let targets = self.core.membership.members.iter()
            .filter(|elem| *elem != &self.core.id)
            .chain(self.core.membership.non_voters.iter())
            .collect::<Vec<_>>();
        for target in targets {
            // Build & spawn a replication stream for the target member.
            let replstream = ReplicationStream::new(
                self.core.id, *target, self.core.current_term, self.core.config.clone(),
                self.core.last_log_index, self.core.last_log_term, self.core.commit_index,
                self.core.network.clone(), self.core.storage.clone(), self.replicationtx.clone(),
            );
            let state = ReplicationState{match_index: self.core.last_log_index, is_at_line_rate: true, replstream, remove_after_commit: None};
            self.nodes.insert(*target, state);
        }

        // Setup state as leader.
        self.core.last_heartbeat = None;
        self.core.next_election_timeout = None;
        self.core.update_current_leader(UpdateCurrentLeader::ThisNode);
        self.core.report_metrics(State::Leader);

        // Per §8, commit an initial entry as part of becoming the cluster leader.
        self.commit_initial_leader_entry().await?;

        loop {
            if !self.core.target_state.is_leader() {
                for node in self.nodes.values() {
                    let _ = node.replstream.repltx.send(RaftEvent::Terminate);
                }
                return Ok(());
            }
            tokio::select!{
                Some((rpc, tx)) = self.core.rx_append_entries.next() => {
                    let res = self.core.handle_append_entries_request(rpc).await;
                    let _ = tx.send(res);
                }
                Some((rpc, tx)) = self.core.rx_vote.next() => {
                    let res = self.core.handle_vote_request(rpc).await;
                    let _ = tx.send(res);
                }
                Some((rpc, tx)) = self.core.rx_install_snapshot.next() => {
                    let res = self.core.handle_install_snapshot_request(rpc).await;
                    let _ = tx.send(res);
                }
                Some(update) = self.core.rx_compaction.next() => self.core.update_snapshot_state(update),
                Some((rpc, tx)) = self.core.rx_client.next() => self.handle_client_request(rpc, tx).await,
                Some((rpc, tx)) = self.core.rx_init.next() => self.core.reject_init_with_config(rpc, tx),
                Some((rpc, tx)) = self.core.rx_propose.next() => {
                    let res = match self.handle_propose_config_change(rpc).await {
                        Ok(res) => match res.await {
                            Ok(_) => Ok(()),
                            Err(err) => Err(err),
                        }
                        Err(err) => Err(err),
                    };
                    let _ = tx.send(res);
                }
                Some(Ok(res)) = self.joint_consensus_cb.next() => {
                    match res {
                        Ok(clientres) => self.handle_joint_consensus_committed(clientres).await?,
                        Err(err) => if let Some(cb) = self.propose_config_change_cb.take() {
                            let _ = cb.send(Err(err));
                        }
                    }
                }
                Some(Ok(res)) = self.uniform_consensus_cb.next() => {
                    match res {
                        Ok(clientres) => {
                            let final_res = self.handle_uniform_consensus_committed(clientres).await;
                            if let Some(cb) = self.propose_config_change_cb.take() {
                                let _ = cb.send(final_res.map_err(From::from));
                            }
                        }
                        Err(err) => if let Some(cb) = self.propose_config_change_cb.take() {
                            let _ = cb.send(Err(err));
                        }
                    }
                }
                Some(event) = self.replicationrx.next() => self.handle_replica_event(event).await,
            }
        }
    }
}

/// A struct tracking the state of a replication stream from the perspective of the Raft actor.
struct ReplicationState<D: AppData> {
    pub match_index: u64,
    pub is_at_line_rate: bool,
    pub remove_after_commit: Option<u64>,
    pub replstream: ReplicationStream<D>,
}

pub enum ConsensusState {
    /// The cluster consensus is uniform; not in a joint consensus state.
    Uniform,
    /// The cluster is in a joint consensus state and is syncing new nodes.
    Joint {
        /// The new nodes which are being synced.
        new_nodes_being_synced: Vec<NodeId>,
        /// A bool indicating if the associated config which started this join consensus has yet been comitted.
        ///
        /// NOTE: when a new leader is elected, it will initialize this value to false, and then
        /// update this value to true once the new leader's blank payload has been committed.
        is_committed: bool,
    }
}

impl ConsensusState {
    /// Check the current state to determine if it is in joint consensus, and if it is safe to finalize the joint consensus.
    ///
    /// The return value will be true if:
    /// 1. this object currently represents a joint consensus state.
    /// 2. the corresponding config for this consensus state has been committed to the cluster.
    /// 3. all new nodes being added to the cluster have been synced.
    pub fn is_joint_consensus_safe_to_finalize(&self) -> bool {
        match self {
            ConsensusState::Joint{is_committed, new_nodes_being_synced}
                if *is_committed && new_nodes_being_synced.len() == 0 => true,
            _ => false,
        }
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////////////////////////

/// Volatile state specific to a Raft node in candidate state.
struct CandidateState<'a, D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> {
    core: &'a mut RaftCore<D, R, E, N, S>,
    /// The number of votes which have been granted by peer nodes.
    votes_granted: u64,
    /// The number of votes needed in order to become the Raft leader.
    votes_needed: u64,
}

impl<'a, D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> CandidateState<'a, D, R, E, N, S> {
    pub(self) fn new(core: &'a mut RaftCore<D, R, E, N, S>) -> Self {
        Self{core, votes_granted: 0, votes_needed: 0}
    }

    /// Run the candidate loop.
    #[tracing::instrument(level="debug", skip(self), fields(id=self.core.id, raft_state="candidate"))]
    pub(self) async fn run(mut self) -> RaftResult<(), E> {
        // Each iteration of the outer loop represents a new term.
        loop {
            // Setup initial state per term.
            self.votes_granted = 1; // We must vote for ourselves per the Raft spec.
            self.votes_needed = ((self.core.membership.members.len() / 2) + 1) as u64; // Just need a majority.

            // Setup new term.
            self.core.update_next_election_timeout(); // Generates a new rand value within range.
            self.core.current_term += 1;
            self.core.voted_for = Some(self.core.id);
            self.core.update_current_leader(UpdateCurrentLeader::Unknown);
            self.core.save_hard_state().await?;
            self.core.report_metrics(State::Candidate);

            // Send RPCs to all members in parallel.
            let mut pending_votes = self.spawn_parallel_vote_requests();

            // Inner processing loop for this Raft state.
            loop {
                if !self.core.target_state.is_candidate() {
                    return Ok(());
                }

                let mut timeout_fut = delay_until(self.core.get_next_election_timeout());
                tokio::select!{
                    _ = &mut timeout_fut => break, // This election has timed-out. Break to outer loop, which starts a new term.
                    Some((rpc, tx)) = self.core.rx_append_entries.next() => {
                        let res = self.core.handle_append_entries_request(rpc).await;
                        let _ = tx.send(res);
                    }
                    Some((rpc, tx)) = self.core.rx_vote.next() => {
                        let res = self.core.handle_vote_request(rpc).await;
                        let _ = tx.send(res);
                    }
                    Some((rpc, tx)) = self.core.rx_install_snapshot.next() => {
                        let res = self.core.handle_install_snapshot_request(rpc).await;
                        let _ = tx.send(res);
                    }
                    Some((rpc, tx)) = self.core.rx_client.next() => self.core.forward_client_request(rpc, tx),
                    Some((rpc, tx)) = self.core.rx_init.next() => self.core.reject_init_with_config(rpc, tx),
                    Some((rpc, tx)) = self.core.rx_propose.next() => self.core.reject_propose_config_change(rpc, tx),
                    Some(update) = self.core.rx_compaction.next() => self.core.update_snapshot_state(update),
                    Some((res, peer)) = pending_votes.recv() => self.handle_vote_response(res, peer).await?,
                }
            }
        }
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////////////////////////

/// Volatile state specific to a Raft node in follower state.
pub struct FollowerState<'a, D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> {
    core: &'a mut RaftCore<D, R, E, N, S>,
}

impl<'a, D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> FollowerState<'a, D, R, E, N, S> {
    pub(self) fn new(core: &'a mut RaftCore<D, R, E, N, S>) -> Self {
        Self{core}
    }

    /// Run the follower loop.
    #[tracing::instrument(level="debug", skip(self), fields(id=self.core.id, raft_state="follower"))]
    pub(self) async fn run(self) -> RaftResult<(), E> {
        self.core.report_metrics(State::Follower);
        loop {
            if !self.core.target_state.is_follower() {
                return Ok(());
            }

            let mut election_timeout = delay_until(self.core.get_next_election_timeout()); // Value is updated as heartbeats are received.
            tokio::select!{
                // If an election timeout is hit, then we need to transition to candidate.
                _ = &mut election_timeout => self.core.set_target_state(TargetState::Candidate),
                Some((rpc, tx)) = self.core.rx_append_entries.next() => {
                    let res = self.core.handle_append_entries_request(rpc).await;
                    let _ = tx.send(res);
                }
                Some((rpc, tx)) = self.core.rx_vote.next() => {
                    let res = self.core.handle_vote_request(rpc).await;
                    let _ = tx.send(res);
                }
                Some((rpc, tx)) = self.core.rx_install_snapshot.next() => {
                    let res = self.core.handle_install_snapshot_request(rpc).await;
                    let _ = tx.send(res);
                }
                Some(update) = self.core.rx_compaction.next() => self.core.update_snapshot_state(update),
                Some((rpc, tx)) = self.core.rx_client.next() => self.core.forward_client_request(rpc, tx),
                Some((rpc, tx)) = self.core.rx_init.next() => self.core.reject_init_with_config(rpc, tx),
                Some((rpc, tx)) = self.core.rx_propose.next() => self.core.reject_propose_config_change(rpc, tx),
            }
        }
    }
}

///////////////////////////////////////////////////////////////////////////////////////////////////
///////////////////////////////////////////////////////////////////////////////////////////////////

/// Volatile state specific to a Raft node in non-voter state.
pub struct NonVoterState<'a, D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> {
    core: &'a mut RaftCore<D, R, E, N, S>,
}

impl<'a, D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> NonVoterState<'a, D, R, E, N, S> {
    pub(self) fn new(core: &'a mut RaftCore<D, R, E, N, S>) -> Self {
        Self{core}
    }

    /// Run the non-voter loop.
    #[tracing::instrument(level="debug", skip(self), fields(id=self.core.id, raft_state="non-voter"))]
    pub(self) async fn run(mut self) -> RaftResult<(), E> {
        self.core.report_metrics(State::NonVoter);
        loop {
            if !self.core.target_state.is_non_voter() {
                return Ok(());
            }
            tokio::select!{
                Some((rpc, tx)) = self.core.rx_append_entries.next() => {
                    let res = self.core.handle_append_entries_request(rpc).await;
                    let _ = tx.send(res);
                }
                Some((rpc, tx)) = self.core.rx_vote.next() => {
                    let res = self.core.handle_vote_request(rpc).await;
                    let _ = tx.send(res);
                }
                Some((rpc, tx)) = self.core.rx_install_snapshot.next() => {
                    let res = self.core.handle_install_snapshot_request(rpc).await;
                    let _ = tx.send(res);
                }
                Some(update) = self.core.rx_compaction.next() => self.core.update_snapshot_state(update),
                Some((rpc, tx)) = self.core.rx_client.next() => self.core.forward_client_request(rpc, tx),
                Some((rpc, tx)) = self.core.rx_init.next() => {
                    let res = self.handle_init_with_config(rpc).await;
                    let _ = tx.send(res);
                }
                Some((rpc, tx)) = self.core.rx_propose.next() => self.core.reject_propose_config_change(rpc, tx),
            }
        }
    }
}
