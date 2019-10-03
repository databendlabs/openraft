//! The Raft actor's module and its associated logic.

mod admin;
mod append_entries;
mod apply_logs;
mod client;
mod install_snapshot;
mod replication;
mod state;
mod vote;

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use actix::prelude::*;
use futures::sync::{mpsc};
use log::{error};

use crate::{
    AppData, AppError, NodeId,
    common::{ApplyLogsTask, DependencyAddr, UpdateCurrentLeader},
    config::Config,
    messages::{ClientPayload, MembershipConfig},
    metrics::{RaftMetrics, State},
    network::RaftNetwork,
    raft::state::{CandidateState, FollowerState, LeaderState, RaftState, ReplicationState, SnapshotState},
    replication::{ReplicationStream, RSTerminate},
    storage::{GetInitialState, GetLogEntries, HardState, InitialState, RaftStorage, SaveHardState},
};

const FATAL_ACTIX_MAILBOX_ERR: &str = "Fatal actix MailboxError while communicating with Raft dependency. Raft is shutting down.";
const FATAL_STORAGE_ERR: &str = "Fatal storage error encountered which can not be recovered from. Stopping Raft node.";

//////////////////////////////////////////////////////////////////////////////////////////////////
// Raft //////////////////////////////////////////////////////////////////////////////////////////

/// An actor which implements the Raft protocol's core business logic.
///
/// For more information on the Raft protocol, see the specification here:
/// https://raft.github.io/raft.pdf (**pdf warning**).
///
/// The beginning of §5, the spec has a condensed summary of the Raft consensus algorithm. This
/// crate, and especially this actor, attempts to follow the terminology and nomenclature used
/// there as precisely as possible to aid in understanding this system.
///
/// ### api
/// This actor's API is broken up into 3 different layers, all based on message handling. In order
/// to effectively use this actor, only these 3 layers need to be considered.
///
/// #### network
/// The network interface of the parent application is responsible for providing a conduit to
/// exchange the various messages types defined in this system. The `RaftNetwork` trait defines
/// the interface needed for being able to allow Raft cluster members to be able to communicate
/// with each other. In addition to the `RaftNetwork` trait, applications are expected to provide
/// and interface for their clients to be able to submit data which needs to be managed by Raft.
///
/// ##### raft rpc messages
/// These are Raft request PRCs coming from other nodes of the cluster. They are defined in the
/// `messages` module of this crate. They are `AppendEntriesRequest`, `VoteRequest` &
/// `InstallSnapshotRequest`. This actor will use the `RaftNetwork` impl of the parent application
/// to send RPCs to other nodes.
///
/// The application's networking layer must decode these message types and pass them over to the
/// appropriate handler on this type, await the response, and then send the response back over the
/// wire to the caller.
///
/// ##### client request messages
/// These are messages coming from an application's clients, represented by the
/// `messages::ClientPayload` type. When the message type's handler is called a future will be
/// returned which will resolve with the appropriate response type. Only data mutating messages
/// should ever need to go through Raft. The contentsof these messages are entirely specific to
/// your application.
///
/// #### storage
/// The storage interface is typically going to be the most involved as this is where your
/// application really exists. SQL, NoSQL, mutable, immutable, KV, append only ... whatever your
/// application's data model, this is where it comes to life.
///
/// The storage interface is defined by the `RaftStorage` trait. Depending on the data storage
/// system being used, the actor my be sync or async. It just needs to implement handlers for
/// the needed actix message types.
///
/// #### admin
/// These are admin commands which may be issued to a Raft node in order to influence it in ways
/// outside of the normal Raft lifecycle. Dynamic membership changes and cluster initialization
/// are the main commands of this layer.
pub struct Raft<D: AppData, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, E>> {
    /// This node's ID.
    id: NodeId,
    /// This node's runtime config.
    config: Arc<Config>,
    /// The cluster's current membership configuration.
    membership: MembershipConfig,
    /// The current state of this Raft node.
    state: RaftState<D, E, N, S>,
    /// The address of the actor responsible for implementing the `RaftNetwork` interface.
    network: Addr<N>,
    /// The address of the actor responsible for implementing the `RaftStorage` interface.
    storage: Addr<S::Actor>,
    /// The address of the actor responsible for recieving metrics output from this Node.
    metrics: Recipient<RaftMetrics>,

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
    /// Is initialized to 0, increases monotonically following the `commit_index` as logs are
    /// applied to the state machine (via the storage interface).
    last_applied: u64,
    /// The current term.
    ///
    /// Is initialized to 0 on first boot, and increases monotonically. This is normally based on
    /// the leader's term which is communicated to other members via the AppendEntries protocol,
    /// but this may also be incremented when a follower becomes a candidate.
    current_term: u64,
    /// The ID of the current leader of the Raft cluster.
    ///
    /// This value is kept up-to-date based on a very simple algorithm, which is the only way to
    /// do so reasonably using only the canonical Raft RPCs described in the spec. When a new
    /// leader comes to power, it will send AppendEntries RPCs to establish its leadership. When
    /// such an RPC is observed with a newer term, this value will be updated. This value will be
    /// set to `None` when a newer term is observed in any other way.
    current_leader: Option<NodeId>,
    /// The ID of the candidate which received this node's vote for the current term.
    ///
    /// Each server will vote for at most one candidate in a given term, on a
    /// first-come-first-served basis. See §5.4.1 for additional restriction on votes.
    voted_for: Option<NodeId>,

    /// The index of the last log to be appended.
    last_log_index: u64,
    /// The term of the last log to be appended.
    last_log_term: u64,

    /// A flag to indicate if this system is currently appending logs.
    is_appending_logs: bool,
    /// The entrypoint to the pipeline of logs which need to be applied to the state machine.
    apply_logs_pipeline: mpsc::UnboundedSender<ApplyLogsTask<D, E>>,
    /// The receiving end of the pipeline for applying logs. This is moved out and spawned when Raft starts.
    _apply_logs_pipeline_receiver: Option<mpsc::UnboundedReceiver<ApplyLogsTask<D, E>>>,

    /// A handle to the election timeout callback.
    election_timeout: Option<actix::SpawnHandle>,
    /// The currently scheduled election timeout.
    election_timeout_stamp: Option<Instant>,
}

impl<D: AppData, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, E>> Raft<D, E, N, S> {
    /// Create a new Raft instance.
    ///
    /// This actor will need to be started after instantiation, which must be done within a
    /// running actix system.
    pub fn new(id: NodeId, config: Config, network: Addr<N>, storage: Addr<S::Actor>, metrics: Recipient<RaftMetrics>) -> Self {
        let state = RaftState::Initializing;
        let config = Arc::new(config);
        let (tx, rx) = mpsc::unbounded();
        let membership = MembershipConfig{is_in_joint_consensus: false, members: vec![id], non_voters: vec![], removing: vec![]};
        Self{
            id, config, membership, state, network, storage, metrics,
            commit_index: 0, last_applied: 0,
            current_term: 0, current_leader: None, voted_for: None,
            last_log_index: 0, last_log_term: 0,
            is_appending_logs: false,
            apply_logs_pipeline: tx, _apply_logs_pipeline_receiver: Some(rx),
            election_timeout: None, election_timeout_stamp: None,
        }
    }

    /// Transition to the Raft non-voter state.
    fn become_non_voter(&mut self, ctx: &mut Context<Self>) {
        // Cleanup previous state.
        self.cleanup_state(ctx);

        // Ensure there is no election timeout.
        self.election_timeout_stamp = None;
        if let Some(handle) = self.election_timeout.take() {
            ctx.cancel_future(handle);
        }

        // Perform the transition.
        self.state = RaftState::NonVoter;
        self.report_metrics(ctx);
    }

    /// Transition to the Raft follower state.
    fn become_follower(&mut self, ctx: &mut Context<Self>) {
        // Don't transition to follower state if the cluster has this node configured as a non-voter.
        if !self.membership.contains(&self.id) || self.membership.non_voters.contains(&self.id) {
            return;
        }

        // Cleanup previous state.
        self.cleanup_state(ctx);

        // Ensure we have an election timeout loop running.
        if self.election_timeout.is_none() {
            self.update_election_timeout(ctx);
        }

        // Perform the transition.
        self.state = RaftState::Follower(FollowerState::default());
        self.report_metrics(ctx);
    }

    /// Transition to the Raft candidate state and start a new election campaign, per §5.2.
    ///
    /// As part of an election campaign, a follower increments its current term and transitions to
    /// candidate state, it then votes for itself (will then save its hard state) and issues
    /// RequestVote RPCs in parallel to each of the other nodes in the cluster.
    ///
    /// A candidate remains in the candidate state until one of three things happens:
    ///
    /// 1. It wins the election.
    /// 2. Another server establishes itself as leader.
    /// 3. A period of time goes by with no winner.
    ///
    /// (1) a candidate wins an election if it receives votes from a majority of the servers
    /// in the full cluster for the same term. Each server will vote for at most one candidate in
    /// a given term, on a first-come-first-served basis (§5.4 adds an additional restriction on
    /// votes). The majority rule ensures that at most one candidate can win the election for a
    /// particular term. Once a candidate wins an election, it becomes leader. It then sends
    /// heartbeat messages to all of the other servers to establish its authority and prevent new
    /// elections.
    ///
    /// (2) While waiting for votes, a candidate may receive an AppendEntries RPC from another
    /// server claiming to be leader. If the leader’s term in the RPC is at least as large as the
    /// candidate’s current term, then the candidate recognizes the leader as legitimate and
    /// returns to follower state. If the term in the RPC is smaller than the candidate’s current
    /// term, then the candidate rejects the RPC and continues in candidate state.
    ///
    /// (3) The third possible outcome is that a candidate neither wins nor loses the election: if
    /// many followers become candidates at the same time, votes could be split so that no
    /// candidate obtains a majority. When this happens, each candidate will time out and start a
    /// new election by incrementing its term and initiating another round of RequestVote RPCs.
    /// The randomization of election timeouts per node helps to avoid this issue.
    fn become_candidate(&mut self, ctx: &mut Context<Self>) {
        // Cleanup previous state.
        self.cleanup_state(ctx);

        // Setup new term.
        self.current_term += 1;
        self.voted_for = Some(self.id);
        self.save_hard_state(ctx);

        // Send RPCs to all members in parallel.
        let mut requests = BTreeMap::new();
        let peers = self.membership.members.iter().filter(|member| *member != &self.id).map(|e| *e).collect::<Vec<_>>();
        for member in peers {
            let f = self.request_vote(ctx, member);
            let handle = ctx.spawn(f);
            requests.insert(member, handle);
        }

        // Update the election timeout.
        self.update_election_timeout(ctx);

        // Update Raft state as candidate.
        let votes_granted = 1; // We must vote for ourselves per the Raft spec.
        let votes_needed = ((self.membership.members.len() / 2) + 1) as u64; // Just need a majority.
        self.state = RaftState::Candidate(CandidateState{requests, votes_granted, votes_needed});
        self.report_metrics(ctx);
    }

    /// Transition to the Raft leader state.
    ///
    /// Once a node becomes the Raft cluster leader, its behavior will be a bit different. Upon
    /// election:
    ///
    /// - Each cluster member gets a `ReplicationStream` actor spawned. Addr is retained.
    /// - Initial AppendEntries RPCs (heartbeats) are sent to each cluster member, and is repeated
    /// during idle periods to prevent election timeouts, per §5.2. This is handled by the
    /// `ReplicationStream` actors.
    /// - A new blank log entry is generated and committed to the cluster in order to ensure that
    /// there are no unapplied entries from the last term, per the end of §8. This blank entry is
    /// used to ensure that a joint consensus config, if present, has been properly committed to
    /// the cluster.
    ///
    /// See the `ClientRpcIn` handler for more details on the write path for client requests.
    fn become_leader(&mut self, ctx: &mut Context<Self>) {
        // Cleanup previous state & ensure we've cancelled the election timeout system.
        self.cleanup_state(ctx);
        if let Some(handle) = self.election_timeout {
            ctx.cancel_future(handle);
        }

        // Prep new leader state.
        let (client_request_queue, client_request_receiver) = mpsc::unbounded();
        let mut new_state = LeaderState::new(client_request_queue, &self.membership);

        // Spawn stream which consumes client RPCs.
        ctx.spawn(fut::wrap_stream(client_request_receiver)
            .and_then(|msg, act: &mut Self, ctx| act.process_client_rpc(ctx, msg))
            .finish());

        // Spawn new replication stream actors.
        let targets = self.membership.members.iter().filter(|elem| *elem != &self.id)
            .chain(self.membership.non_voters.iter());
        for target in targets {
            // Build the replication stream for the target member.
            let rs = ReplicationStream::new(
                self.id, *target, self.current_term, self.config.clone(),
                self.last_log_index, self.last_log_term, self.commit_index,
                ctx.address(), self.network.clone(), self.storage.clone().recipient::<GetLogEntries<D, E>>(),
            );
            let addr = rs.start(); // Start the actor on the same thread.

            // Retain the addr of the replication stream.
            let state = ReplicationState{match_index: self.last_log_index, is_at_line_rate: true, addr, remove_after_commit: None};
            new_state.nodes.insert(*target, state);
        }

        // Initialize new state as leader.
        self.state = RaftState::Leader(new_state);
        self.update_current_leader(ctx, UpdateCurrentLeader::ThisNode);
        self.report_metrics(ctx);

        // Commit a new blank entry to the cluster to guard against stale-reads, per §8.
        // If the cluster has just formed, and the current index is 0, then commit the current config.
        let payload = if self.last_log_index == 0 {
            ClientPayload::new_config(self.membership.clone())
        } else {
            ClientPayload::new_blank_payload()
        };
        ctx.spawn(fut::wrap_future(ctx.address().send(payload))
            .map_err(|_, _, _| ())
            .and_then(|res, _, _| fut::result(res.map_err(|_| ())))
            // In the case that there was a stale record and it was a joint consensus
            // finalization, ensure it is handled properly.
            .and_then(|res, act: &mut Self, ctx| act.handle_joint_consensus_finalization(ctx, res))
        );
    }

    /// Clean up the current Raft state.
    ///
    /// This will typically be called before a state transition takes place.
    fn cleanup_state(&mut self, ctx: &mut Context<Self>) {
        match &mut self.state {
            RaftState::Follower(inner) => {
                inner.snapshot_state = SnapshotState::Idle;
            }
            RaftState::Candidate(inner) => {
                for handle in inner.cleanup() {
                    ctx.cancel_future(handle);
                }
            }
            RaftState::Leader(inner) => {
                inner.nodes.values().for_each(|rsstate| {
                    let _ = rsstate.addr.do_send(RSTerminate);
                });
            }
            _ => (),
        }
    }

    /// Perform the initialization routine for the Raft node.
    ///
    /// If this node has configuration present from being online previously, then this node will
    /// begin a standard lifecycle as a follower. If this node is pristine, then it will wait in
    /// standby mode.
    ///
    /// ### previous state | follower
    /// If the node has previous state, then there are a few cases to account for.
    ///
    /// If the node has been offline for some time and was removed from the cluster, no problem.
    /// Any RPCs sent from this node will be rejected until it is added to the cluster. Once it is
    /// added to the cluster again, the standard Raft protocol will resume as normal.
    ///
    /// If the node went down only very briefly, then it should immediately start receiving
    /// heartbeats and resume as normal, else it will start an election if it doesn't receive any
    /// heartbeats from the leader per normal Raft protocol.
    ///
    /// If the node was running standalone, it will win the election and resume as a standalone.
    ///
    /// ### pristine state | standby
    /// While in standby mode, the Raft leader of the current cluster may discover this node and
    /// add it to the cluster. In such a case, it will begin receiving heartbeats from the leader
    /// and business proceeds as usual.
    ///
    /// If there is no current cluster, while in standby mode, the node may receive an admin
    /// command instructing it to campaign with a specific config, or to begin operating as the
    /// leader of a standalone cluster.
    fn initialize(&mut self, ctx: &mut Context<Self>, state: InitialState) {
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

        // Spawn the stream for applying logs to the state machine. This will always be `Some` here, never after.
        if let Some(rx) = self._apply_logs_pipeline_receiver.take() {
            ctx.spawn(fut::wrap_stream(rx)
                .and_then(|msg, act: &mut Self, ctx| act.process_apply_logs_task(ctx, msg))
                .finish());
        }

        // Set initial state based on state recovered from disk.
        let is_only_configured_member = self.membership.len() == 1 && self.membership.contains(&self.id);
        if !is_only_configured_member || &self.last_log_index != &u64::min_value() {
            self.state = RaftState::Follower(FollowerState::default());
            self.update_election_timeout(ctx);
        } else {
            self.state = RaftState::NonVoter;
        }

        // Begin reporting metrics.
        ctx.run_interval(self.config.metrics_rate.clone(), |act, ctx| act.report_metrics(ctx));
    }

    /// Transform and log an actix MailboxError.
    ///
    /// This method treats the error as being fatal, as Raft can not function properly if the
    /// `RaftNetowrk` & `RaftStorage` interfaces are returning mailbox errors. This method will
    /// shutdown the Raft actor.
    fn map_fatal_actix_messaging_error(&mut self, ctx: &mut Context<Self>, err: actix::MailboxError, dep: DependencyAddr) {
        error!("{} {:?} {:?}", FATAL_ACTIX_MAILBOX_ERR, dep, err);
        ctx.terminate();
    }

    /// Transform an log the result of a `RaftStorage` interaction.
    ///
    /// This method assumes that a storage error observed here is non-recoverable. As such, the
    /// Raft node will be instructed to stop. If such behavior is not needed, then don't use this
    /// interface.
    fn map_fatal_storage_result<T>(&mut self, ctx: &mut Context<Self>, res: Result<T, E>) -> impl ActorFuture<Actor=Self, Item=T, Error=()> {
        let res = res.map_err(|err| {
            error!("{} {:?}", FATAL_STORAGE_ERR, err);
            ctx.terminate();
        });
        fut::result(res)
    }

    /// Report a metrics payload on the current state of the Raft node.
    fn report_metrics(&mut self, _: &mut Context<Self>) {
        let state = match &self.state {
            RaftState::NonVoter => State::NonVoter,
            RaftState::Follower(_) => State::Follower,
            RaftState::Candidate(_) => State::Candidate,
            RaftState::Leader(_) => State::Leader,
            _ => return,
        };
        let _ = self.metrics.do_send(RaftMetrics{
            id: self.id, state, current_term: self.current_term,
            last_log_index: self.last_log_index,
            last_applied: self.last_applied,
            current_leader: self.current_leader,
            membership_config: self.membership.clone(),
        }).map_err(|err| {
            error!("Error reporting metrics. {}", err);
        });
    }

    /// Save the Raft node's current hard state to disk.
    ///
    /// DEPRECATED: use `save_hard_state_async`.
    fn save_hard_state(&mut self, ctx: &mut Context<Self>) {
        let hs = HardState{current_term: self.current_term, voted_for: self.voted_for, membership: self.membership.clone()};
        let f = fut::wrap_future(self.storage.send::<SaveHardState<E>>(SaveHardState::new(hs)))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res));

        ctx.spawn(f);
    }

    /// Save the Raft node's current hard state to disk.
    fn save_hard_state_async(&mut self, _: &mut Context<Self>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        let hs = HardState{current_term: self.current_term, voted_for: self.voted_for, membership: self.membership.clone()};
        fut::wrap_future(self.storage.send::<SaveHardState<E>>(SaveHardState::new(hs)))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
    }

    /// Update the value of the `current_leader` property.
    ///
    /// NOTE WELL: there was previously a bit of log encapsulated here related to forwarding
    /// requests to leaders and such. In order to more closely mirror the Raft spec and allow apps
    /// to determine how they want to handle forwarding client requests to leaders, that logic was
    /// removed and this handler has thus been greatly simplified. We are keeping it as is in case
    /// we need to add some additional logic here.
    fn update_current_leader(&mut self, _: &mut Context<Self>, update: UpdateCurrentLeader) {
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
    fn update_current_term(&mut self, new_term: u64, voted_for: Option<NodeId>) {
        if new_term > self.current_term {
            self.current_term = new_term;
            self.voted_for = voted_for;
        }
    }

    /// Update the election timeout process.
    ///
    /// This will schedule a new interval job based on the configured election timeout. The
    /// interval job will check to see if a campaign should be started based on when the last
    /// heartbeat was received from the Raft leader or a candidate.
    ///
    /// The election timeout stamp will be updated everytime this node receives an RPC from the
    /// leader as well as any time a candidate node sends a RequestVote RPC if it is a
    /// valid vote request.
    fn update_election_timeout(&mut self, ctx: &mut Context<Self>) {
        // Don't update if the cluster has this node configured as a non-voter.
        if !self.membership.contains(&self.id) || self.membership.non_voters.contains(&self.id) {
            return;
        }

        // Cancel any current election timeout before spawning a new one.
        if let Some(handle) = self.election_timeout.take() {
            ctx.cancel_future(handle);
        }

        let timeout = Duration::from_millis(self.config.election_timeout_millis);
        self.election_timeout_stamp = Some(Instant::now() + timeout.clone());
        self.election_timeout = Some(ctx.run_interval(timeout, |act, ctx| {
            if let Some(stamp) = &act.election_timeout_stamp {
                if &Instant::now() >= stamp {
                    act.become_candidate(ctx)
                }
            }
        }));
    }

    /// Update the election timeout stamp, typically due to receiving a heartbeat from the Raft leader.
    fn update_election_timeout_stamp(&mut self) {
        self.election_timeout_stamp = Some(Instant::now() + Duration::from_millis(self.config.election_timeout_millis));
    }

    /// Update the node's current membership config.
    ///
    /// NOTE WELL: if a leader is stepping down, it should not call this method, as it will cause
    /// the node to transition out of leader state before it can commit the config entry.
    fn update_membership(&mut self, ctx: &mut Context<Self>, cfg: MembershipConfig) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        self.membership = cfg;

        // If the given config does not contain this node's ID, it means one of the following:
        // - the node is currently a non-voter and is replicating an old config to which it has
        // not yet been added.
        // - the node has been removed from the cluster. The parent application can observe the
        // transition to the non-voter state as a signal for when it is safe to shutdown a node
        // being removed.
        if !self.membership.contains(&self.id) {
            self.become_non_voter(ctx);
        } else if self.state.is_non_voter() && self.membership.members.contains(&self.id) {
            // The node is a NonVoter and the new config has it configured as a normal member.
            // Transition to follower.
            self.become_follower(ctx);
        }

        self.save_hard_state_async(ctx)
    }
}

impl<D: AppData, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, E>> Actor for Raft<D, E, N, S> {
    type Context = Context<Self>;

    /// The initialization routine for this actor.
    fn started(&mut self, ctx: &mut Self::Context) {
        // Fetch the node's initial state from the storage actor & initialize.
        let f = fut::wrap_future(self.storage.send::<GetInitialState<E>>(GetInitialState::new()))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
            .map(|state, act, ctx| act.initialize(ctx, state));
        ctx.spawn(f);
    }
}
