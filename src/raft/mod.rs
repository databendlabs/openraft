//! The Raft actor's module and its associated logic.

mod append_entries;
mod client;
pub mod common;
mod install_snapshot;
mod replication;
mod vote;

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::Duration,
};

use actix::prelude::*;
use futures::sync::{mpsc};
use log::{debug, error};

use crate::{
    NodeId, AppError,
    config::Config,
    messages::{ClientError},
    metrics::{RaftMetrics, State},
    network::RaftNetwork,
    raft::common::{AwaitingCommitted, ClientPayloadWithTx, DependencyAddr, UpdateCurrentLeader},
    replication::{ReplicationStream, RSTerminate},
    storage::{GetInitialState, HardState, InitialState, RaftStorage, SaveHardState},
};

const FATAL_ACTIX_MAILBOX_ERR: &str = "Fatal actix MailboxError while communicating with Raft dependency. Raft is shutting down.";
const FATAL_STORAGE_ERR: &str = "Fatal storage error encountered which can not be recovered from. Stopping Raft node.";

//////////////////////////////////////////////////////////////////////////////////////////////////
// RaftState /////////////////////////////////////////////////////////////////////////////////////

/// The state of the Raft node.
enum RaftState<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> {
    /// A non-standard Raft state indicating that the node is initializing.
    Initializing,
    /// A non-standard Raft state indicating that the node is awaiting an admin command to begin.
    ///
    /// The Raft node will only be in this state when it comes online for the very first time
    /// without any state recovered from disk. In such a state, the parent application may have
    /// this new node added to an already running cluster, may have the node start as the leader
    /// of a new standalone cluster, or have the node initialize with a specific config.
    ///
    /// This state gives control over Raft's initial cluster formation and node startup to the
    /// application which is using this system.
    Standby,
    /// The node is actively replicating logs from the leader.
    ///
    /// The node is passive when it is in this state. It issues no requests on its own but simply
    /// responds to requests from leaders and candidates.
    Follower,
    /// The node has detected an election timeout so is requesting votes to become leader.
    ///
    /// This state wraps struct which tracks outstanding requests to peers for requesting votes
    /// along with the number of votes granted.
    Candidate(CandidateState),
    /// The node is actively functioning as the Raft cluster leader.
    ///
    /// The leader handles all client requests. If a client contacts a follower, the follower must
    /// redirects it to the leader.
    Leader(LeaderState<E, N, S>),
}

/// Volatile state specific to the Raft leader.
///
/// This state is reinitialized after an election.
struct LeaderState<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> {
    /// A mapping of node IDs the replication state of the target node.
    pub nodes: BTreeMap<NodeId, ReplicationState<E, N, S>>,
    /// A queue of client requests to be processed.
    pub client_request_queue: mpsc::UnboundedSender<ClientPayloadWithTx<E>>,
    /// The current client RPC which is awaiting to be comitted.
    pub awaiting_committed: Option<AwaitingCommitted<E>>,
}

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> LeaderState<E, N, S> {
    /// Create a new instance.
    pub fn new(tx: mpsc::UnboundedSender<ClientPayloadWithTx<E>>) -> Self {
        Self{nodes: Default::default(), client_request_queue: tx, awaiting_committed: None}
    }
}

/// A struct tracking the state of a replication stream from the perspective of the Raft actor.
struct ReplicationState<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> {
    pub match_index: u64,
    pub is_at_line_rate: bool,
    pub addr: Addr<ReplicationStream<E, N, S>>,
}

/// Volatile state specific to a Raft node in candidate state.
struct CandidateState {
    /// Current outstanding requests to peer nodes by node ID.
    requests: BTreeMap<NodeId, SpawnHandle>,
    /// The number of votes which have been granted by peer nodes.
    votes_granted: u64,
    /// The number of votes needed in order to become the Raft leader.
    votes_needed: u64,
}

impl CandidateState {
    /// Cleanup state resources.
    pub(self) fn cleanup(&mut self) -> Vec<SpawnHandle> {
        let keys = self.requests.keys().map(|k| *k).collect::<Vec<_>>();
        let mut handles = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(f) = self.requests.remove(&key) {
                handles.push(f);
            }
        }
        handles
    }
}

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
/// This actor's API is broken up into 4 different layers, all based on message handling. In order
/// to effectively use this actor, only these 4 interfaces need to considered.
///
/// #### raft request messages
/// These are Raft request PRCs coming from other nodes of the cluster. This interface is
/// implemented as an `actix::Handler`, so a future will be returned which will resolve with the
/// appropriate response type.
///
/// Typically your application's networking layer will decode these message types and simply pass
/// them over to this handler, await the response, and then send the response back over the wire
/// to the caller. However, this is entirely application specific, so at the end of the day, all
/// one needs to do is send the message to this actor, await the response, and then handle the
/// response as needed by your application.
///
/// #### client request messages
/// These are messages coming from your application's clients. This interface is implemented as an
/// `actix::Handler`, so a future will be returned which will resolve with the appropriate
/// response type. Only data mutating messages should ever need to go through Raft. The contents
/// of these messages are entirely specific to your application.
///
/// #### outbound raft request
/// These are messages originating from the Raft actor which are destined for some peer node of
/// the Raft cluster. When the Raft actor is instantiated, an `actix::Recipient` must be supplied
/// which is expected to handle the networking layer logic of actually sending the message to the
/// target peer.
///
/// The networking layer is application specific, so no constraints are put in place in terms of
/// how your application's nodes are to communicate with each other. The only thing that is
/// required is that the message be sent and the response be delivered back.
///
/// Per the Raft spec, message delivery failure from a leader to a follower is to be retried
/// indefinitely, so that is how this actor is implemented.
///
/// #### storage
/// The storage interface is typically going to be the most involved as this is where your
/// application really exists. SQL, NoSQL, mutable, immutable, KV, append only ... whatever your
/// application's data model, this is where it comes to life.
///
/// The storage interface is provided as an `actix::Addr<S: RaftStorage>`. The generic type `S`
/// must implement the `RaftStorage` trait, which is composed of a series of actix message
/// handling traits.
///
/// Depending on the data storage system being used, the actor my be sync or async. It just needs
/// to implement handlers for the needed actix message types.
///
/// Note that currently, when this actor encounters an error from the storage layer, it will stop.
/// The rest of the system may remain online as long as is needed, but this actor will stop in
/// order to avoid data corruption or other such issues.
pub struct Raft<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> {
    /// This node's ID.
    id: NodeId,
    /// This node's runtime config.
    config: Arc<Config>,
    /// All currently known members of the Raft cluster.
    members: Vec<NodeId>,
    /// The current state of this Raft node.
    state: RaftState<E, N, S>,
    /// The address of the actor responsible for implementing the `RaftNetwork` interface.
    network: Addr<N>,
    /// The address of the actor responsible for implementing the `RaftStorage` interface.
    storage: Addr<S>,
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
    /// A flag to indicate if this system is currently applying logs to the state machine.
    is_applying_logs_to_state_machine: bool,

    /// A handle to the election timeout callback.
    election_timeout: Option<actix::SpawnHandle>,

    /// A buffer of client RPC requests to be forwarded to the Raft leader once it is known.
    forwarding: Vec<ClientPayloadWithTx<E>>,
}

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Raft<E, N, S> {
    /// Create a new Raft instance.
    ///
    /// This actor will need to be started after instantiation, which must be done within a
    /// running actix system.
    pub fn new(id: NodeId, config: Config, network: Addr<N>, storage: Addr<S>, metrics: Recipient<RaftMetrics>) -> Self {
        let state = RaftState::Initializing;
        let config = Arc::new(config);
        Self{
            id, config, members: vec![id], state, network, storage, metrics,
            commit_index: 0, last_applied: 0,
            current_term: 0, current_leader: None, voted_for: None,
            last_log_index: 0, last_log_term: 0,
            is_appending_logs: false, is_applying_logs_to_state_machine: false,
            election_timeout: None, forwarding: vec![],
        }
    }

    /// Transition to the Raft follower state.
    fn become_follower(&mut self, ctx: &mut Context<Self>) {
        // No-op if we were already in follower state.
        if let &RaftState::Follower = &self.state {
            return;
        }
        debug!("Raft {} is transitioning to follower state.", &self.id);

        // Cleanup previous state.
        self.cleanup_state(ctx);

        // Ensure we have an election timeout loop running.
        if self.election_timeout.is_none() {
            self.update_election_timeout(ctx);
        }

        // Perform the transition.
        self.state = RaftState::Follower;
        self.report_metrics(ctx);
        debug!("Raft {} has finished transition to follower state.", &self.id);
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
        debug!("Raft {} is transitioning to candidate state.", &self.id);
        // Cleanup previous state.
        self.cleanup_state(ctx);

        // Setup new term.
        self.current_term += 1;
        self.voted_for = Some(self.id);
        self.save_hard_state(ctx);

        // Send RPCs to all members in parallel.
        let mut requests = BTreeMap::new();
        let peers = self.members.clone().into_iter().filter(|member| member != &self.id).collect::<Vec<_>>();
        for member in peers {
            let f = self.request_vote(ctx, member);
            let handle = ctx.spawn(f);
            requests.insert(member, handle);
        }

        // Update Raft state as candidate.
        let votes_granted = 1; // We must vote for ourselves per the Raft spec.
        let votes_needed = ((self.members.len() / 2) + 1) as u64; // Just need a majority.
        self.state = RaftState::Candidate(CandidateState{requests, votes_granted, votes_needed});
        self.report_metrics(ctx);
        debug!("Raft {} has finished transition to candidate state.", &self.id);
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
    ///
    /// See the `ClientRpcIn` handler for more details on the write path for client requests.
    fn become_leader(&mut self, ctx: &mut Context<Self>) {
        debug!("Raft {} is transitioning to leader state.", &self.id);
        // Cleanup previous state & ensure we've cancelled the election timeout system.
        self.cleanup_state(ctx);
        if let Some(handle) = self.election_timeout {
            ctx.cancel_future(handle);
        }

        // Prep new leader state.
        let (tx, rx) = mpsc::unbounded();
        let tx0 = tx.clone();
        let mut new_state = LeaderState::new(tx);

        // Spawn stream which consumes client RPCs.
        ctx.spawn(fut::wrap_stream(rx)
            .and_then(|msg, act: &mut Self, ctx| act.process_client_rpc(ctx, msg))
            .finish());

        // Spawn new replication stream actors.
        for target in self.members.iter().filter(|elem| *elem != &self.id) {
            // Build the replication stream for the target member.
            let rs = ReplicationStream::new(
                self.id, *target, self.current_term, self.config.clone(),
                self.last_log_index, self.last_log_term, self.commit_index,
                ctx.address(), self.network.clone(), self.storage.clone().recipient(),
            );
            let addr = rs.start(); // Start the actor on the same thread.

            // Retain the addr of the replication stream.
            let state = ReplicationState{match_index: self.last_log_index, is_at_line_rate: true, addr};
            new_state.nodes.insert(*target, state);
        }

        // Initialize new state as leader.
        self.state = RaftState::Leader(new_state);
        self.update_current_leader(ctx, UpdateCurrentLeader::ThisNode(tx0));
        self.report_metrics(ctx);
        debug!("Raft {} has finished transition to leader state.", &self.id);
    }

    /// Clean up the current Raft state.
    ///
    /// This will typically be called before a state transition is to take place.
    fn cleanup_state(&mut self, ctx: &mut Context<Self>) {
        match &mut self.state {
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
        debug!("Raft {} initialized with state {:?}.", &self.id, &state);
        self.last_log_index = state.last_log_index;
        self.last_log_term = state.last_log_term;
        self.current_term = state.hard_state.current_term;
        self.voted_for = state.hard_state.voted_for;
        self.members = state.hard_state.members;
        self.last_applied = state.last_applied_log;

        // Set initial state based on state recovered from disk.
        let is_only_configured_member = self.members.len() == 1 && self.members.contains(&self.id);
        if !is_only_configured_member || &self.last_log_index != &u64::min_value() {
            debug!("Raft {} initialized as follower.", &self.id);
            self.state = RaftState::Follower;
            self.update_election_timeout(ctx);
        } else {
            debug!("Raft {} initialized to standby state.", &self.id);
            self.state = RaftState::Standby;
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
        ctx.stop();
        // TODO: may need to perform some additional cleanup. If so, encapsulate the shutdown call.
    }

    /// Transform an log the result of a `RaftStorage` interaction.
    ///
    /// This method assumes that a storage error observed here is non-recoverable. As such, the
    /// Raft node will be instructed to stop. If such behavior is not needed, then don't use this
    /// interface.
    fn map_fatal_storage_result<T>(&mut self, ctx: &mut Context<Self>, res: Result<T, E>) -> impl ActorFuture<Actor=Self, Item=T, Error=()> {
        let res = res.map_err(|err| {
            error!("{} {:?}", FATAL_STORAGE_ERR, err);
            ctx.stop();
        });
        fut::result(res)
    }

    /// Report a metrics payload on the current state of the Raft node.
    fn report_metrics(&mut self, _: &mut Context<Self>) {
        let state = match &self.state {
            RaftState::Standby => State::Standby,
            RaftState::Follower => State::Follower,
            RaftState::Candidate(_) => State::Candidate,
            RaftState::Leader(_) => State::Leader,
            _ => return,
        };
        let _ = self.metrics.do_send(RaftMetrics{
            id: self.id, state, current_term: self.current_term,
            last_log_index: self.last_log_index,
            last_applied: self.last_applied,
            current_leader: self.current_leader,
        }).map_err(|err| {
            error!("Error reporting metrics. {}", err);
        });
    }

    /// Save the Raft node's current hard state to disk.
    fn save_hard_state(&mut self, ctx: &mut Context<Self>) {
        let hs = HardState{current_term: self.current_term, voted_for: self.voted_for, members: self.members.clone()};
        let f = fut::wrap_future(self.storage.send(SaveHardState::new(hs)))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res));

        ctx.spawn(f);
    }

    /// Update the value of the `current_leader` property.
    ///
    /// Depending on the update type, any buffered client requests will be processed on this node
    /// if it is the new leader, or they will be sent to the target node by ID, or nothing will
    /// happen if the leader is unknown.
    fn update_current_leader(&mut self, ctx: &mut Context<Self>, update: UpdateCurrentLeader<E>) {
        match update {
            // Handle condition where client requests were buffered & this node has become the leader.
            UpdateCurrentLeader::ThisNode(tx) => {
                self.current_leader = Some(self.id);
                let outbound: Vec<_> = self.forwarding.drain(..).collect();
                for buffered in outbound {
                    // Queue the buffered client request for processing on this node.
                    let _ = tx.unbounded_send(buffered).map_err(|err| {
                        // If the channel is closed for some reason, respond with a forwarding error.
                        // Practically speaking, this should never take place as the unbounded sender
                        // provided in `UpdateCurrentLeader::ThisNode(_)` should be freshly allocated.
                        let _ = err.into_inner().tx.send(Err(ClientError::Internal));
                    });
                }
            }
            // Handle condition where client requests were buffered & a different node has become leader.
            UpdateCurrentLeader::OtherNode(target) => {
                self.current_leader = Some(target);
                for buffered in self.forwarding.drain(..) {
                    let ClientPayloadWithTx{tx, rpc: mut outbound} = buffered;
                    outbound.target = target;
                    let f = fut::wrap_future(self.network.send(outbound))
                        .map_err(|err, _, _| {
                            error!("Error forwarding client request. {:?}", err);
                            ClientError::Internal
                        })
                        .and_then(|res, _, _| fut::result(res))
                        .then(move |res, _, _| {
                            let _ = tx.send(res).map_err(|_| ());
                            fut::ok(())
                        });
                    ctx.spawn(f);
                }
            }
            UpdateCurrentLeader::Unknown => {
                self.current_leader = None;
            },
        }
    }

    /// Update the election timeout process.
    ///
    /// This will run the nodes election timeout mechanism to ensure that elections are held if
    /// too much time passes before hearing from a leader or a candidate.
    ///
    /// The election timeout will be updated everytime this node receives an RPC from the leader
    /// as well as any time a candidate node sends a RequestVote RPC. We reset on candidate RPCs
    /// iff the RPC is a valid vote request.
    fn update_election_timeout(&mut self, ctx: &mut Context<Self>) {
        // Cancel any current election timeout before spawning a new one.
        if let Some(handle) = self.election_timeout.take() {
            ctx.cancel_future(handle);
        }
        let timeout = Duration::from_millis(self.config.election_timeout_millis);
        self.election_timeout = Some(ctx.run_later(timeout, |act, ctx| act.become_candidate(ctx)));
    }
}

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Actor for Raft<E, N, S> {
    type Context = Context<Self>;

    /// The initialization routine for this actor.
    fn started(&mut self, ctx: &mut Self::Context) {
        debug!("Starting Raft node with ID {}.", &self.id);
        // Fetch the node's initial state from the storage actor & initialize.
        let f = fut::wrap_future(self.storage.send(GetInitialState::new()))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
            .map(|state, act, ctx| act.initialize(ctx, state));
        ctx.spawn(f);
    }
}
