//! A module encapsulating the core `Raft` actor and its logic.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::Duration,
};

use actix::prelude::*;
use futures::sync::{mpsc, oneshot};
use log::{error, warn};
use serde::{Deserialize, Serialize};

use crate::{
    config::Config,
    error::{ClientRpcError, RaftRpcError, StorageResult},
    proto,
    replication::{
        ReplicationStream, RSReplicate,
        RSNeedsSnapshot, RSNeedsSnapshotResponse,
        RSRateUpdate, RSRevertToFollower, RSUpdateMatchIndex,
    },
    storage::{
        self, AppendLogEntries, AppendLogEntriesData, ApplyEntriesToStateMachine,
        ApplyEntriesToStateMachineData, CreateSnapshot, GetCurrentSnapshot,
        GetInitialState, GetLogEntries, HardState, InitialState, InstallSnapshot,
        InstallSnapshotChunk, RaftStorage, SaveHardState,
    },
};

const ACTIX_MESSAGING_ERR: &str = "An internal actix messaging error was encountered.";
const CLIENT_RPC_CHAN_ERR: &str = "Client RPC channel was unexpectedly closed.";

/// A Raft cluster node's ID.
pub type NodeId = u64;

//////////////////////////////////////////////////////////////////////////////////////////////////
// NodeState /////////////////////////////////////////////////////////////////////////////////////

/// The state of the Raft node.
enum NodeState {
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
    Leader(LeaderState),
}

/// Volatile state specific to the Raft leader.
///
/// This state is reinitialized after an election.
struct LeaderState {
    /// A mapping of node IDs the replication state of the target node.
    pub nodes: BTreeMap<NodeId, ReplicationState>,
    /// A queue of client requests to be processed.
    pub client_request_queue: mpsc::UnboundedSender<ClientRpcInWithTx>,
    /// The current client RPC which is awaiting to be comitted.
    pub awaiting_committed: Option<AwaitingCommitted>,
}

impl LeaderState {
    /// Create a new instance.
    pub fn new(tx: mpsc::UnboundedSender<ClientRpcInWithTx>) -> Self {
        Self{nodes: Default::default(), client_request_queue: tx, awaiting_committed: None}
    }
}

/// A struct tracking the state of a replication stream from the perspective of the Raft actor.
struct ReplicationState {
    pub match_index: u64,
    pub is_at_line_rate: bool,
    pub addr: Recipient<RSReplicate>,
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
pub struct Raft<S: RaftStorage>  {
    /// This node's ID.
    id: NodeId,
    /// This node's runtime config.
    config: Arc<Config>,
    /// All currently known members of the Raft cluster.
    members: Vec<NodeId>,
    /// The current state of this Raft node.
    state: NodeState,
    /// An output channel for sending Raft request messages to peers.
    out: actix::Recipient<RaftRpcOut>,
    /// An output channel for forwarding client requests to the leader.
    forward: actix::Recipient<ClientRpcOut>,
    /// The address of the actor responsible for implementing the `RaftStorage` interface.
    storage: Addr<S>,

    /// The index of the highest log entry known to be committed cluster-wide.
    ///
    /// The definition of a committed log is that the leader which has created the log has
    /// successfully replicated the log to a majority of the cluster. This value is only ever
    /// updated by way of an AppendEntries RPC from the leader. If a node is the leader, it will
    /// update this value as new entries have been successfully replicated to a majority of the
    /// cluster.
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
    forwarding: Vec<ClientRpcInWithTx>,
}

impl<S: RaftStorage> Raft<S> {
    /// Create a new Raft instance.
    ///
    /// This actor will need to be started after instantiation, which must be done within a
    /// running actix system.
    ///
    /// TODO: add an example on how to create and start an instance.
    pub fn new(id: NodeId, config: Config, out: actix::Recipient<RaftRpcOut>, forward: actix::Recipient<ClientRpcOut>, storage: Addr<S>) -> Self {
        let state = NodeState::Initializing;
        let config = Arc::new(config);
        Self{
            id, config, members: vec![id], state, out, forward, storage,
            commit_index: 0, last_applied: 0,
            current_term: 0, current_leader: None, voted_for: None,
            last_log_index: 0, last_log_term: 0,
            is_appending_logs: false, is_applying_logs_to_state_machine: false,
            election_timeout: None, forwarding: vec![],
        }
    }

    /// Append the given entries to the log.
    ///
    /// This routine also encapsulates all logic which must be performed related to appending log
    /// entries.
    ///
    /// One important piece of logic to note here is the handling of config change entries. Per
    /// the Raft spec in §6:
    ///
    /// > Once a given server adds the new configuration entry to its log, it uses that
    /// > configuration for all future decisions (a server always uses the latest configuration in
    /// > its log, regardless of whether the entry is committed).
    ///
    /// This routine will extract the most recent (the latter most) entry in the given payload of
    /// entries which is a config change entry and will update the node's member state based on
    /// that entry.
    fn append_log_entries(
        &mut self, ctx: &mut Context<Self>, entries: Vec<proto::Entry>,
    ) -> impl ActorFuture<Actor=Self, Item=AppendLogEntriesData, Error=RaftRpcError> {
        // If we are already eppending entries, then abort this operation.
        if self.is_appending_logs {
            return fut::Either::A(fut::FutureResult::from(Err(RaftRpcError::AppendEntriesAlreadyInProgress)));
        }

        // Check the given entries for any config changes and take the most recent.
        use proto::entry::EntryType;
        let last_conf_change = entries.iter().filter_map(|ent| match &ent.entry_type {
            Some(EntryType::ConfigChange(conf)) => Some(conf),
            _ => None,
        }).last();
        if let Some(conf) = last_conf_change {
            // Update membership info & apply hard state.
            self.members = conf.members.clone();
            self.save_hard_state(ctx);
        }

        self.is_appending_logs = true;
        fut::Either::B(fut::wrap_future(self.storage.send(AppendLogEntries(entries)))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error_raft_rpc(err))
            .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
            .map(|data, act, _| {
                act.last_log_index = data.index;
                act.last_log_term = data.term;
                data
            })
            .then(|res, act, _| {
                act.is_appending_logs = false;
                fut::FutureResult::from(res)
            }))
    }

    /// Begin the process of applying logs to the state machine.
    fn apply_logs_to_state_machine(&mut self, ctx: &mut Context<Self>) {
        // If logs are already being applied, do nothing.
        if self.is_applying_logs_to_state_machine {
            return;
        }

        // Fetch the series of entries which must be applied to the state machine.
        self.is_applying_logs_to_state_machine = true;
        let f = fut::wrap_future(self.storage.send(GetLogEntries{start: self.last_applied, stop: self.commit_index + 1}))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error_raft_rpc(err))
            .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))

            // Send the entries over to the storage engine to be applied to the state machine.
            .and_then(|entries, act, _| {
                fut::wrap_future(act.storage.send(ApplyEntriesToStateMachine(entries)))
                    .map_err(|err, _: &mut Self, _| Self::map_messaging_error_raft_rpc(err))
                    .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
            })

            // Update self to reflect progress on applying logs to the state machine.
            .and_then(|data, act, _| {
                act.last_applied = data.index;
                act.is_applying_logs_to_state_machine = false;
                fut::FutureResult::from(Ok(()))
            })

            // Log any errors which may have come from the process of applying log entries.
            .map_err(|err, _, _| {
                error!("{}", err)
            });
        let _ = ctx.spawn(f);
    }

    /// Transition to the Raft follower state.
    fn become_follower(&mut self, ctx: &mut Context<Self>) {
        // No-op if we were already in follower state.
        if let &NodeState::Follower = &self.state {
            return;
        }

        // Cleanup previous state.
        self.cleanup_state(ctx);

        // Ensure we have an election timeout loop running.
        if self.election_timeout.is_none() {
            self.update_election_timeout(ctx);
        }

        // Perform the transition.
        self.state = NodeState::Follower;
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
        let peers = self.members.clone().into_iter().filter(|member| member != &self.id).collect::<Vec<_>>();
        for member in peers {
            let f = self.request_vote(ctx, member, self.id, self.last_log_index, self.last_log_term);
            let handle = ctx.spawn(f);
            requests.insert(member, handle);
        }

        // Update Raft state as candidate.
        let votes_granted = 1; // We must vote for ourselves per the Raft spec.
        let votes_needed = ((self.members.len() / 2) + 1) as u64; // Just need a majority.
        self.state = NodeState::Candidate(CandidateState{requests, votes_granted, votes_needed});
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
                ctx.address(), self.out.clone(), self.storage.clone().recipient(),
            );
            let addr = rs.start(); // Start the actor on the same thread.

            // Retain the addr of the replication stream.
            let state = ReplicationState{match_index: self.last_log_index, is_at_line_rate: true, addr: addr.recipient()};
            new_state.nodes.insert(*target, state);
        }

        // Initialize new state as leader.
        self.state = NodeState::Leader(new_state);
        self.update_current_leader(ctx, UpdateCurrentLeader::ThisNode(tx0));
    }

    /// Clean up the current Raft state.
    ///
    /// This will typically be called before a state transition is to take place.
    fn cleanup_state(&mut self, ctx: &mut Context<Self>) {
        match &mut self.state {
            NodeState::Candidate(inner) => {
                for handle in inner.cleanup() {
                    ctx.cancel_future(handle);
                }
            }
            _ => (),
        }
    }

    /// Forward the given client request to the last known leader of the cluster.
    fn forward_request_to_leader(&mut self, ctx: &mut Context<Self>, msg: ClientRpcInWithTx) {
        // If we have a currently known leader, then forward the request to it.
        if let Some(target) = &self.current_leader {
            let ClientRpcInWithTx{tx, rpc} = msg;
            let rpc_out = ClientRpcOut{target: *target, rpc};
            let f = fut::wrap_future(self.forward.send(rpc_out))
                .map_err(|err, _, _| {
                    error!("Error forwarding client request to leader. {:?}", err);
                    ClientRpcError::ForwardingError
                })
                .and_then(|res, _, _| fut::FutureResult::from(res))
                .then(move |res, _, _| {
                    let _ = tx.send(res).map_err(|_| ());
                    fut::FutureResult::from(Ok(()))
                });
            ctx.spawn(f);
        } else {
            // We don't know the ID of the current leader, or one has not been elected yet. Buffer the request.
            self.forwarding.push(msg);
        }
    }

    /// Handle requests from Raft leader to append log entries.
    ///
    /// This method implements the append entries algorithm and upholds all of the safety checks
    /// detailed in §5.3.
    ///
    /// The essential goal of this algorithm is that the receiver (the node on which this method
    /// is being executed) must find the exact entry in its log specified by the RPC's last index
    /// and last term fields, and then begin writing the new entries thereafter.
    ///
    /// When the receiver can not find the entry specified in the RPC's prev index & prev term
    /// fields, it will respond with a failure to the leader. **This implementation of Raft
    /// includes the _conflicting term_ optimization** which is intended to reduce the number of
    /// rejected append entries RPCs from followers which are lagging behind, which is detailed in
    /// §5.3. In such cases, if the Raft cluster is configured with a snapshot policy other than
    /// `Disabled`, the leader will make a determination if an `InstallSnapshot` RPC should be
    /// sent to this node.
    ///
    /// In Raft, the leader handles inconsistencies by forcing the followers’ logs to duplicate
    /// its own. This means that conflicting entries in follower logs will be overwritten with
    /// entries from the leader’s log. §5.4 details the safety of this protocol. It is important
    /// to note that logs which are _committed_ will not be overwritten. This is a critical
    /// feature of Raft.
    ///
    /// Raft also gurantees that only logs which have been comitted may be applied to the state
    /// machine, which ensures that there will never be a case where a log needs to be reverted
    /// after being applied to the state machine.
    ///
    /// #### inconsistency example
    /// Followers may receive valid append entries requests from leaders, append them, respond,
    /// and before the leader is able to replicate the entries to a majority of nodes, the leader
    /// may die, a new leader may be elected which does not have the same entries, as they were
    /// not replicated to a majority of followers, and the new leader will proceeed to overwrite
    /// the inconsistent entries.
    fn handle_append_entries_request(
        &mut self, ctx: &mut Context<Self>, msg: proto::AppendEntriesRequest,
    ) -> impl ActorFuture<Actor=Self, Item=proto::AppendEntriesResponse, Error=RaftRpcError> {
        // Don't interact with non-cluster members.
        if !self.members.contains(&msg.leader_id) {
            return fut::Either::A(
                fut::Either::A(fut::err(RaftRpcError::RPCFromUnknownNode))
            );
        }

        // If message's term is less than most recent term, then we do not honor the request.
        if &msg.term < &self.current_term {
            return fut::Either::A(
                fut::Either::A(fut::ok(proto::AppendEntriesResponse{term: self.current_term, success: false, conflict_opt: None}))
            );
        }

        // Update election timeout & ensure we are in the follower state. Update current term if needed.
        self.update_election_timeout(ctx);
        if &msg.term > &self.current_term {
            self.become_follower(ctx);
            self.current_term = msg.term;
            self.update_current_leader(ctx, UpdateCurrentLeader::OtherNode(msg.leader_id));
            self.save_hard_state(ctx);
        }

        // Kick off process of applying logs to state machine based on `msg.leader_commit`.
        self.commit_index = msg.leader_commit; // The value for `self.commit_index` is only updated here.
        if &self.commit_index > &self.last_applied {
            self.apply_logs_to_state_machine(ctx);
        }

        // If the AppendEntries RPC has no entries, then this was just a heartbeat.
        if msg.entries.len() == 0 {
            return fut::Either::A(
                fut::Either::A(fut::ok(proto::AppendEntriesResponse{term: self.current_term, success: true, conflict_opt: None}))
            );
        }

        // If RPC's `prev_log_index` is 0, or the RPC's previous log info matches the local
        // previous log info, then replication is g2g.
        let term = self.current_term;
        if &msg.prev_log_index == &u64::min_value() || (&msg.prev_log_index == &self.last_log_index && &msg.prev_log_term == &self.last_log_term) {
            return fut::Either::A(fut::Either::B(
                self.append_log_entries(ctx, msg.entries)
                    .map(move |_, _, _| {
                        proto::AppendEntriesResponse{term, success: true, conflict_opt: None}
                    })));
        }

        // Previous log info doesn't immediately line up, so perform log consistency check and
        // proceed based on its result.
        fut::Either::B(self.log_consistency_check(ctx, msg.prev_log_index, msg.prev_log_term)
            .and_then(move |res, act, ctx| match res {
                Some(conflict_opt) => fut::Either::A(fut::FutureResult::from(Ok(
                    proto::AppendEntriesResponse{term, success: false, conflict_opt: Some(conflict_opt)}
                ))),
                None => fut::Either::B(act.append_log_entries(ctx, msg.entries)
                    .map(move |_, _, _| {
                        proto::AppendEntriesResponse{term, success: true, conflict_opt: None}
                    })),
            }))
    }

    /// Handle requests from peers to cast a vote for a new leader.
    fn handle_vote_request(&mut self, ctx: &mut Context<Self>, msg: proto::VoteRequest) -> Result<proto::VoteResponse, RaftRpcError> {
        // Don't interact with non-cluster members.
        if !self.members.contains(&msg.candidate_id) {
            return Err(RaftRpcError::RPCFromUnknownNode);
        }

        // If candidate's current term is less than this nodes current term, reject.
        if &msg.term < &self.current_term {
            return Ok(proto::VoteResponse{term: self.current_term, vote_granted: false});
        }

        // If candidate's log is not at least as up-to-date as this node, then reject.
        if &msg.last_log_term < &self.last_log_term || &msg.last_log_index < &self.last_log_index {
            return Ok(proto::VoteResponse{term: self.current_term, vote_granted: false});
        }

        // Candidate's log is up-to-date so handle voting conditions.

        // If term is newer than current term, cast vote.
        if &msg.term > &self.current_term {
            self.current_term = msg.term;
            self.voted_for = Some(msg.candidate_id);
            self.save_hard_state(ctx);
            self.update_election_timeout(ctx);
            return Ok(proto::VoteResponse{term: self.current_term, vote_granted: true});
        }

        // Term is the same as current term. This will be rare, but could come about from some error conditions.
        match &self.voted_for {
            // This node has already voted for the candidate.
            Some(candidate_id) if candidate_id == &msg.candidate_id => {
                self.update_election_timeout(ctx);
                Ok(proto::VoteResponse{term: self.current_term, vote_granted: true})
            }
            // This node has already voted for a different candidate.
            Some(_) => Ok(proto::VoteResponse{term: self.current_term, vote_granted: false}),
            // This node has not already voted, so vote for the candidate.
            None => {
                self.voted_for = Some(msg.candidate_id);
                self.save_hard_state(ctx);
                self.update_election_timeout(ctx);
                Ok(proto::VoteResponse{term: self.current_term, vote_granted: true})
            },
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
        self.members = state.hard_state.members;
        self.last_applied = state.last_applied_log;

        // Set initial state based on state recovered from disk.
        let is_only_configured_member = self.members.len() == 1 && self.members.contains(&self.id);
        if is_only_configured_member || &self.last_log_index != &u64::min_value() {
            self.state = NodeState::Follower;
            self.update_election_timeout(ctx);
        } else {
            self.state = NodeState::Standby;
        }
    }

    /// Perform the AppendEntries RPC consistency check.
    ///
    /// If the log entry at the specified index does not exist, the most recent entry in the log
    /// will be used to build and return a `ConflictOpt` struct to be sent back to the leader.
    ///
    /// If The log entry at the specified index does exist, but the terms to no match up, this
    /// implementation will fetch the last 50 entries from the given index, and will use the
    /// earliest entry from the log which is still in the given term to build a `ConflictOpt`
    /// struct to be sent back to the leader.
    ///
    /// If everyhing checks out, a `None` value will be returned and log replication may continue.
    fn log_consistency_check(
        &mut self, _: &mut Context<Self>, index: u64, term: u64,
    ) -> impl ActorFuture<Actor=Self, Item=Option<proto::ConflictOpt>, Error=RaftRpcError> {
        let storage = self.storage.clone();
        fut::wrap_future(self.storage.send(GetLogEntries{start: index, stop: index}))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error_raft_rpc(err))
            .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
            .and_then(move |res, act, _| {
                match res.last() {
                    // The target entry was not found. This can only mean that we don't have the
                    // specified index yet. Use the last known index & term.
                    None => fut::Either::A(fut::FutureResult::from(Ok(Some(proto::ConflictOpt{
                        term: act.last_log_term,
                        index: act.last_log_index,
                    })))),
                    // The target entry was found. Compare its term with target term to ensure
                    // everything is consistent.
                    Some(entry) => {
                        let entry_term = entry.term;
                        if entry_term == term {
                            // Everything checks out. We're g2g.
                            fut::Either::A(fut::FutureResult::from(Ok(None)))
                        } else {
                            // Logs are inconsistent. Fetch the last 50 logs, and use the last
                            // entry of that payload which is still in the target term for
                            // conflict optimization.
                            fut::Either::B(fut::wrap_future(storage.send(GetLogEntries{start: index, stop: index}))
                                .map_err(|err, _: &mut Self, _| Self::map_messaging_error_raft_rpc(err))
                                .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
                                .and_then(move |res, _, _| {
                                    match res.into_iter().filter(|entry| entry.term == term).nth(0) {
                                        Some(entry) => fut::FutureResult::from(Ok(Some(proto::ConflictOpt{
                                            term: entry.term,
                                            index: entry.index,
                                        }))),
                                        None => fut::FutureResult::from(Ok(Some(proto::ConflictOpt{
                                            term: entry_term,
                                            index: index,
                                        }))),
                                    }
                                }))
                        }
                    }
                }
            })
    }

    /// A simple mapping function to log and transform an `actix::MailboxError` for client RPCs.
    fn map_messaging_error_client_rpc(err: actix::MailboxError) -> ClientRpcError {
        error!("{} {}", ACTIX_MESSAGING_ERR, err);
        ClientRpcError::InternalMessagingError
    }

    /// A simple mapping function to log and transform an `actix::MailboxError` for Raft RPCs.
    fn map_messaging_error_raft_rpc(err: actix::MailboxError) -> RaftRpcError {
        error!("{} {}", ACTIX_MESSAGING_ERR, err);
        RaftRpcError::InternalMessagingError
    }

    /// Map an `actix::MailboxError` from a call to the storage engine.
    ///
    /// This will stop the Raft node, as communication with the storage engine is required for
    /// proper functionality.
    fn map_messaging_error_storage_engine(&mut self, ctx: &mut Context<Self>, err: actix::MailboxError) {
        error!("Failed to communicate with RaftStorage. Stopping Raft. {:?}", err);
        ctx.stop();
    }

    /// A simple mapping function to transform a `StorageResult` from the storage layer.
    ///
    /// **NOTE WELL:** This method assumes that a storage error observed here is non-recoverable.
    /// As such, the Raft node will be instructed to stop. If such behavior is not needed, then
    /// don't use this interface.
    fn map_storage_result<T>(&mut self, ctx: &mut Context<Self>, res: StorageResult<T>) -> Result<T, RaftRpcError> {
        res.map_err(|err| {
            error!("Storage error encountered which can not be recovered from. Stopping Raft node.");
            ctx.stop();
            RaftRpcError::StorageError(err)
        })
    }

    /// Process the given client RPC, appending it to the log and issuing the needed response.
    ///
    /// This function takes the given RPC, appends its entries to the log, sends the entries out
    /// to the replication streams to be replicated to the cluster followers, after half of the
    /// cluster members have successfully replicated the entries this routine will proceed with
    /// applying the entries to the state machine. Then the next RPC is processed.
    ///
    /// TODO: there is an optimization to be had here:
    /// - after half of the nodes have replicated the entries of the RPC, we can technically begin
    /// processing the next RPC.
    /// - applying entries to the state machine must still be done in order, but we can create a
    /// buffer of entries which will ensure entries are applied in serial order.
    /// - if we use this approach, the RPC along with its entries could be buffered together so
    /// that after the RPC's segment of entries have been applied, responses can be issued for the
    /// RPCs which were submitted with `ResponseMode::Applied`.
    fn process_client_rpc(&mut self, ctx: &mut Context<Self>, msg: ClientRpcInWithTx) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        match &self.state {
            // If node is still leader, continue.
            NodeState::Leader(_) => (),
            // If node is in any other state, then forward the message to the leader.
            _ => {
                self.forward_request_to_leader(ctx, msg);
                return fut::Either::A(fut::FutureResult::from(Ok(())));
            }
        };

        // Transform the entries of the RPC into log entries.
        let mut line_index = self.last_log_index;
        let entries: Vec<_> = msg.rpc.entries.clone().into_iter().map(|data| {
            line_index += 1;
            proto::Entry{
                index: line_index,
                term: self.current_term,
                entry_type: Some(proto::entry::EntryType::Normal(proto::EntryNormal{data})),
            }
        }).collect();

        // Send the entries over to the storage engine.
        self.is_appending_logs = true;
        let f = fut::wrap_future(self.storage.send(AppendLogEntries(entries.clone())))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error_client_rpc(err))
            .and_then(|res, _, _| fut::FutureResult::from(res.map_err(|err| ClientRpcError::StorageError(err))))
            // Handle results from storage engine.
            .then(move |res, act, _| match res {
                Ok(_) => {
                    act.last_log_index = line_index;
                    act.is_appending_logs = false;
                    fut::FutureResult::from(Ok((msg, entries)))
                }
                Err(err) => {
                    let _ = msg.tx.send(Err(err)).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                    fut::FutureResult::from(Err(()))
                }
            })

            // Send logs over for replication.
            .and_then(move |(rpc, entries), act, ctx| {
                // Get a reference to the leader's state, else forward to leader.
                let (state, rx) = match &mut act.state {
                    NodeState::Leader(state) => {
                        let (tx, rx) = oneshot::channel();
                        state.awaiting_committed = Some(AwaitingCommitted{index: line_index, rpc, chan: tx});
                        (state, rx)
                    },
                    _ => {
                        act.forward_request_to_leader(ctx, rpc);
                        return fut::Either::A(fut::FutureResult::from(Err(())));
                    }
                };

                // Send payload over to each replication stream as needed.
                for rs in state.nodes.values() {
                    // Only send full payload over if the target stream is running at line rate.
                    let payload = if rs.is_at_line_rate { entries.clone() } else { Vec::with_capacity(0) };
                    let _ = rs.addr.do_send(RSReplicate{entries: payload, line_index, line_commit: act.commit_index});
                }

                // Resolve this step in the pipeline once the RPC's entries have been comitted accross the cluster.
                fut::Either::B(fut::wrap_future(rx
                    .map(|val| (val, entries))
                    .map_err(|_| ())))
            })
            // The RPC's entries have been committed, handle client response & applying to state machine locally.
            .and_then(move |(rpc, entries), act, _| {
                // If this RPC is configured to wait only for log committed, then respond to client now.
                let tx = if let &ResponseMode::Committed = &rpc.rpc.response_mode {
                    let _ = rpc.tx.send(Ok(())).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                    None
                } else {
                    Some(rpc.tx)
                };

                // Apply entries to state machine.
                act.is_applying_logs_to_state_machine = true;
                fut::wrap_future(act.storage.send(ApplyEntriesToStateMachine(entries)))
                    .map_err(|err, act: &mut Self, ctx| act.map_messaging_error_storage_engine(ctx, err))
                    .and_then(move |res, act, ctx| {
                        let res = res.map_err(|err| {
                            error!("Storage error encountered which can not be recovered from. Stopping Raft node.");
                            ctx.stop();
                            ClientRpcError::StorageError(err)
                        });

                        match res {
                            Ok(_data) => {
                                // Update state after a success operation on the state machine.
                                act.is_applying_logs_to_state_machine = false;
                                act.last_applied = line_index;

                                // If this RPC is configured to wait for applied, then respond to client now.
                                if let Some(tx) = tx {
                                    let _ = tx.send(Ok(())).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                                }
                                fut::FutureResult::from(Ok(()))
                            }
                            Err(err) => {
                                if let Some(tx) = tx {
                                    let _ = tx.send(Err(err)).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                                }
                                fut::FutureResult::from(Err(()))
                            }
                        }
                    })
            });
        fut::Either::B(f)
    }

    /// Request a vote from the the target peer.
    fn request_vote(
        &mut self, _: &mut Context<Self>, target: NodeId, term: u64, log_index: u64, log_term: u64,
    ) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        let rpc = RaftRpcOut{
            target,
            request: proto::RaftRequest::new_vote(proto::VoteRequest::new(term, self.id, log_index, log_term)),
        };
        fut::wrap_future(self.out.send(rpc))
            .map_err(|_, _: &mut Self, _| ())
            .and_then(|res, _, _| match res {
                Ok(inner) => match inner.payload {
                    Some(proto::raft_response::Payload::Vote(res)) => fut::FutureResult::from(Ok(res)),
                    _ => fut::FutureResult::from(Err(())),
                }
                Err(_) => fut::FutureResult::from(Err(())),
            })
            .and_then(|res, act, ctx| {
                // Ensure the node is still in candidate state.
                let state = match &mut act.state {
                    NodeState::Candidate(state) => state,
                    // If this node is not currently in candidate state, then this request is done.
                    _ => return fut::FutureResult::from(Ok(())),
                };

                // If peer's term is greater than current term, revert to follower state.
                if res.term > act.current_term {
                    act.become_follower(ctx);
                    act.current_term = res.term;
                    act.current_leader = None;
                    act.save_hard_state(ctx);
                    return fut::FutureResult::from(Ok(()));
                }

                // If peer granted vote, then update campaign state.
                if res.vote_granted {
                    state.votes_granted += 1;
                    if state.votes_granted >= state.votes_needed {
                        // If the campaign was successful, go into leader state.
                        act.become_leader(ctx);
                    }
                }

                fut::FutureResult::from(Ok(()))
            })
    }

    /// Save the Raft node's current hard state to disk.
    fn save_hard_state(&mut self, ctx: &mut Context<Self>) {
        let hs = HardState{current_term: self.current_term, voted_for: self.voted_for, members: self.members.clone()};
        let f = fut::wrap_future(self.storage.send(SaveHardState(hs)))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error_raft_rpc(err))
            .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
            .map_err(|err, _, _| {
                error!("{}", err)
            });

        ctx.spawn(f);
    }

    /// Update the value of the `current_leader` property.
    ///
    /// Depending on the update type, any buffered client requests will be processed on this node
    /// if it is the new leader, or they will be sent to the target node by ID, or nothing will
    /// happen if the leader is unknown.
    fn update_current_leader(&mut self, ctx: &mut Context<Self>, update: UpdateCurrentLeader) {
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
                        let _ = err.into_inner().tx.send(Err(ClientRpcError::ForwardingError));
                    });
                }
            }
            // Handle condition where client requests were buffered & a different node has become leader.
            UpdateCurrentLeader::OtherNode(target) => {
                self.current_leader = Some(target);
                for buffered in self.forwarding.drain(..) {
                    let ClientRpcInWithTx{tx, rpc} = buffered;
                    let outbound = ClientRpcOut{target, rpc};
                    let f = fut::wrap_future(self.forward.send(outbound))
                        .map_err(|err, _, _| {
                            error!("Error forwarding client request. {:?}", err);
                            ClientRpcError::ForwardingError
                        })
                        .and_then(|res, _, _| fut::FutureResult::from(res))
                        .then(move |res, _, _| {
                            let _ = tx.send(res).map_err(|_| ());
                            fut::FutureResult::from(Ok(()))
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

impl<S: RaftStorage> Actor for Raft<S> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Fetch the node's initial state from the storage actor & initialize.
        let f = fut::wrap_future(self.storage.send(GetInitialState))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error_raft_rpc(err))
            .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
            .map_err(|_, _, _| ())
            .map(|state, act, ctx| act.initialize(ctx, state));

        ctx.spawn(f);
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// Replication Streams ///////////////////////////////////////////////////////////////////////////

// RSRateUpdate //////////////////////////////////////////////////////////////

impl<S: RaftStorage> Handler<RSRateUpdate> for Raft<S> {
    type Result = ();

    /// Handle events from replication streams.
    ///
    /// TODO: finish this up.
    fn handle(&mut self, _msg: RSRateUpdate, _ctx: &mut Self::Context) {
    }
}

// RSNeedsSnapshot ///////////////////////////////////////////////////////////

impl<S: RaftStorage> Handler<RSNeedsSnapshot> for Raft<S> {
    type Result = ResponseActFuture<Self, RSNeedsSnapshotResponse, ()>;

    /// Handle events from replication streams requesting for snapshot info.
    ///
    /// TODO: finish this up.
    fn handle(&mut self, _msg: RSNeedsSnapshot, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::FutureResult::from(Err(())))
    }
}

// RSRevertToFollower ////////////////////////////////////////////////////////

impl<S: RaftStorage> Handler<RSRevertToFollower> for Raft<S> {
    type Result = ();

    /// Handle events from replication streams for when this node needs to revert to follower state.
    ///
    /// TODO: finish this up.
    fn handle(&mut self, _msg: RSRevertToFollower, ctx: &mut Self::Context) {
        self.update_current_leader(ctx, UpdateCurrentLeader::Unknown);
        self.become_follower(ctx);
    }
}

// RSUpdateMatchIndex ////////////////////////////////////////////////////////

impl<S: RaftStorage> Handler<RSUpdateMatchIndex> for Raft<S> {
    type Result = ();

    /// Handle events from a replication stream which updates the target node's match index.
    ///
    /// TODO: finish this up.
    fn handle(&mut self, _msg: RSUpdateMatchIndex, _ctx: &mut Self::Context) {
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RaftRpcOut ////////////////////////////////////////////////////////////////////////////////////

/// An actix message holding a Raft RPC frame to be sent outbound to a target Raft node.
pub struct RaftRpcOut {
    pub target: NodeId,
    pub request: proto::RaftRequest,
}

impl Message for RaftRpcOut {
    type Result = Result<proto::RaftResponse, ()>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RaftRpcIn /////////////////////////////////////////////////////////////////////////////////////

/// An actix::Message wrapping a protobuf RaftRequest.
pub struct RaftRpcIn(pub proto::RaftRequest);

impl Message for RaftRpcIn {
    type Result = Result<proto::RaftResponse, RaftRpcError>;
}

impl<S: RaftStorage> Handler<RaftRpcIn> for Raft<S> {
    type Result = ResponseActFuture<Self, proto::RaftResponse, RaftRpcError>;

    /// Handle inbound Raft request messages.
    ///
    /// Typically the Raft messages sent to this handler will have come from peer nodes of the
    /// Raft cluster, usuall via your application's networking layer. This handler guarantees that
    /// the appropriate Raft response message type will be returned in response according to the
    /// Raft spec.
    ///
    /// ### errors
    /// It is rare that an error will be returned from this handler. The most typical error
    /// conditions are related to storage failures; however that is not the only error case. In
    /// the case of a storage failure however, this actor will go into the stopping state and shut
    /// down.
    ///
    /// On an application level, it would be prudent to fail the associated request when an error
    /// takes place here and have the caller perform a retry.
    fn handle(&mut self, msg: RaftRpcIn, ctx: &mut Self::Context) -> Self::Result {
        // Only handle requests if actor has finished initialization.
        if let &NodeState::Initializing = &self.state {
            warn!("Received RaftRequest before initialization was complete.");
            return Box::new(fut::err(RaftRpcError::Initializing));
        }

        // Unpack the given message and pass to the appropriate handler.
        use proto::raft_request::Payload;
        use proto::raft_response::Payload as ResponsePayload;
        match msg.0.payload {
            Some(Payload::AppendEntries(payload)) => {
                Box::new(self.handle_append_entries_request(ctx, payload)
                    .map(|res, _, _| proto::RaftResponse{
                        payload: Some(ResponsePayload::AppendEntries(res)),
                    }))
            },
            Some(Payload::Vote(payload)) => {
                Box::new(fut::FutureResult::from(self.handle_vote_request(ctx, payload))
                    .map(|res, _, _| proto::RaftResponse{
                        payload: Some(ResponsePayload::Vote(res)),
                    }))
            },
            Some(Payload::InstallSnapshot(_payload)) => {
                // TODO: finish this up.
                Box::new(fut::err(RaftRpcError::Initializing))
            },
            None => {
                warn!("RaftRequest received which had an empty or unknown payload.");
                Box::new(fut::err(RaftRpcError::UnknownRequestReceived))
            }
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientRpcOut //////////////////////////////////////////////////////////////////////////////////

/// A struct representing a client request which must be sent to a target node.
///
/// This typically only comes out from the Raft actor due to needing to forward a client request
/// to the cluster leader.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClientRpcOut {
    pub target: NodeId,
    pub rpc: ClientRpcIn,
}

impl Message for ClientRpcOut {
    type Result = Result<(), ClientRpcError>;
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientRpcIn ///////////////////////////////////////////////////////////////////////////////////

/// An actix message representing a client request to mutate the data of this Raft cluster.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClientRpcIn {
    /// A vector of entries to be appended to the log.
    ///
    /// Each element is vector of bytes corresponding to a standard client request. These will
    /// be appended to the log as normal entries.
    pub entries: Vec<Vec<u8>>,
    /// The response mode for this RPC's corresponding request.
    pub response_mode: ResponseMode,
}

impl Message for ClientRpcIn {
    type Result = Result<(), ClientRpcError>;
}

/// The desired response mode for a client request.
///
/// This value specifies when a client request desires to receive its response from Raft. When
/// `Comitted` is chosen, the client request will receive a response after the request has been
/// successfully replicated to at least half of the nodes in the cluster. This is what the Raft
/// protocol refers to as being comitted.
///
/// When `Applied` is chosen, the client request will receive a response after the request has
/// been successfully committed and successfully applied to the state machine.
///
/// The choice between these two options depends on the requirements related to the request. If
/// the data of the client request payload will need to be read immediately after the response is
/// received, then `Applied` must be used. If there is no requirement that the data must be
/// immediately read after receiving a response, then `Committed` may be used to speed up response
/// times for data mutating requests.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum ResponseMode {
    /// A response will be returned after the request has been committed to the cluster.
    Committed,
    /// A response will be returned  after the request has been applied to the state machine.
    Applied,
}

impl<S: RaftStorage> Handler<ClientRpcIn> for Raft<S> {
    type Result = actix::ResponseActFuture<Self, (), ClientRpcError>;

    /// Handle a client request to mutate the data of the Raft cluster.
    fn handle(&mut self, msg: ClientRpcIn, ctx: &mut Self::Context) -> Self::Result {
        // Wrap the given message for async processing.
        let (tx, rx) = oneshot::channel();
        let with_tx = ClientRpcInWithTx{tx, rpc: msg};

        // Queue the message for processing or forward it along to the leader.
        match &mut self.state {
            NodeState::Leader(state) => {
                let _ = state.client_request_queue.unbounded_send(with_tx).map_err(|_| {
                    error!("Unexpected error while queueing client request for processing.")
                });
            },
            _ => self.forward_request_to_leader(ctx, with_tx),
        };

        // Build a response from the message's channel.
        Box::new(fut::wrap_future(rx)
            .map_err(|err, _, _| {
                error!("Internal client response channel was unexpectedly dropped.");
                ClientRpcError::InternalMessagingError
            })
            .and_then(|res, _, _| fut::FutureResult::from(res)))
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ClientRpcInWithTx /////////////////////////////////////////////////////////////////////////////

struct ClientRpcInWithTx {
    pub tx: oneshot::Sender<Result<(), ClientRpcError>>,
    pub rpc: ClientRpcIn,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// UpdateCurrentLeader ///////////////////////////////////////////////////////////////////////////

/// An enum describing the way the current leader property is to be updated.
enum UpdateCurrentLeader {
    Unknown,
    OtherNode(NodeId),
    ThisNode(mpsc::UnboundedSender<ClientRpcInWithTx>),
}

/// A struct encapsulating an RPC which is awaiting to be committed.
struct AwaitingCommitted {
    /// The index which needs to be comitted for this value to resolve.
    pub index: u64,
    /// The buffered RPC.
    pub rpc: ClientRpcInWithTx,
    /// The chan to be used for resolution once the RPC's index has been comitted.
    pub chan: oneshot::Sender<ClientRpcInWithTx>,
}

// TODO:
// - actix messaging errors from messaging the RaftStorage should cause Raft to stop. This
// functionality is fundamentally required for the system to work.
//
// ### admin commands
// - get AdminCommands setup and implemented.
//
// ### observability
// - ensure that internal state transitions and updates are emitted for host application use. Such
// as NodeState changes, membership changes, errors from async ops.
//
// ### testing
// - setup testing framework to assert accurate behavior of Raft implementation and adherence to
// Raft's safety protocols.
// - all actor based. Transport layer can be a simple message passing mechanism.
// - will probably need to implement MemoryStroage for testing.
