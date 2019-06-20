//! A module encapsulating the core `Raft` actor and its logic.

use std::collections::BTreeMap;

use actix::prelude::*;
use log::{error, warn};

use crate::{
    config::Config,
    proto,
    storage::{self, AppendLogEntries, ApplyEntriesToStateMachine, GetLogEntries, InitialState, RaftStorage},
};

/// An error message for communication errors while attempting to fetch log entries.
const COMM_ERR_FETCH_LOG_ENTRIES: &str = "Error communicating with storage actor for fetching log entries.";

/// An error message for communication errors while attempting to append log entries.
const COMM_ERR_APPEND_LOG_ENTRIES: &str = "Error communicating with storage actor for appending log entries.";

/// A Raft cluster node's ID.
pub type NodeId = u64;

/// The state of the Raft node.
pub enum NodeState {
    /// The node is actively replicating logs from the leader.
    ///
    /// The node is passive when it is in this state. It issues no requests on its own but simply
    /// responds to requests from leaders and candidates.
    Follower,
    /// The node has detected an election timeout so is requesting votes to become leader.
    Candidate,
    /// The node is actively functioning as the Raft cluster leader.
    ///
    /// The leader handles all client requests. If a client contacts a follower, the follower must
    /// redirects it to the leader.
    Leader(LeaderState),
}

/// Volatile state specific to the Raft leader.
///
/// This state is reinitialized after an election.
pub struct LeaderState {
    /// A mapping of node IDs to the index of the next log entry to send.
    ///
    /// Each entry is initialized to leader's last log index + 1. Per the Raft protocol spec,
    /// this value may be decremented as new nodes enter the cluster and need to catch-up.
    ///
    /// When a leader first comes to power, it initializes all `next_index` values to the index
    /// just after the last one in its log.
    ///
    /// If a follower’s log is inconsistent with the leader’s, the AppendEntries consistency check
    /// will fail in the next AppendEntries RPC. After a rejection, the leader decrements
    /// `next_index` and retries the AppendEntries RPC. Eventually `next_index` will reach a point
    /// where the leader and follower logs match. When this happens, AppendEntries will succeed,
    /// which removes any conflicting entries in the follower’s log and appends entries from the
    /// leader’s log (if any). Once AppendEntries succeeds, the follower’s log is consistent with
    /// the leader’s, and it will remain that way for the rest of the term.
    next_index: BTreeMap<NodeId, u64>,
    /// A mapping of node IDs to the highest log entry index known to be replicated thereof.
    ///
    /// Each entry is initialized to 0, and increases per the Raft data replication protocol.
    match_index: BTreeMap<NodeId, u64>,
}

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
pub struct Raft<S: RaftStorage> {
    /// A value indicating if this actor has successfully completed its initialization routine.
    is_initialized: bool,
    /// This node's runtime config.
    config: Config,
    /// This node's ID.
    id: NodeId,
    /// All currently known members of the Raft cluster.
    members: Vec<NodeId>,
    /// The current state of this Raft node.
    state: NodeState,
    /// An output channel for sending Raft request messages to peers.
    out: actix::Recipient<RaftRequest>,
    /// The address of the actor responsible for implementing the `RaftStorage` interface.
    storage: actix::Addr<S>,

    /// The index of the highest log entry known to be committed cluster-wide.
    ///
    /// The definition of a committed log is that the leader which has created the log has
    /// successfully replicated the log to a majority of the cluster. This value is only ever
    /// updated by way of an AppendEntries RPC from the leader.
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
}

impl<S> Raft<S> where S: RaftStorage {
    /// Create a new Raft instance.
    ///
    /// This actor will need to be started after instantiation, which must be done within a
    /// running actix system.
    ///
    /// TODO: add an example on how to create and start an instance.
    pub fn new(id: NodeId, config: Config, out: actix::Recipient<RaftRequest>, storage: Addr<S>) -> Self {
        let state = NodeState::Follower;
        Self{
            is_initialized: false, config,
            id, members: vec![], state, out, storage,
            commit_index: 0, last_applied: 0,
            current_term: 0, voted_for: None,
            last_log_index: 0, last_log_term: 0,
            is_appending_logs: false, is_applying_logs_to_state_machine: false,
        }
    }

    /// Begin the process of applying logs to the state machine.
    fn apply_logs_to_state_machine(&mut self, ctx: &mut Context<Self>) {
        // Fetch the series of entries which must be applied to the state machine.
        self.is_applying_logs_to_state_machine = true;
        let f = fut::wrap_future(self.storage.send(GetLogEntries{start: self.last_applied, stop: self.commit_index + 1}))
            .map_err(|err, _: &mut Raft<S>, _| error!("{}{}", COMM_ERR_FETCH_LOG_ENTRIES, err))
            .and_then(|res, act, ctx| act.storage_engine_result(ctx, res))

            // Send the entries over to the storage engine to be applied to the state machine.
            .and_then(|entries, act, _| {
                fut::wrap_future(act.storage.send(ApplyEntriesToStateMachine(entries)))
                    .map_err(|err, _: &mut Raft<S>, _| error!("{}{}", COMM_ERR_FETCH_LOG_ENTRIES, err))
                    .and_then(|res, act, ctx| act.storage_engine_result(ctx, res))
            })

            // Update self to reflect progress on applying logs to the state machine.
            .and_then(|data, act, _| {
                act.last_applied = data.index;
                act.is_applying_logs_to_state_machine = false;
                fut::FutureResult::from(Ok(()))
            });
        let _ = ctx.spawn(f);
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
    ) -> impl ActorFuture<Actor=Self, Item=Option<proto::ConflictOpt>, Error=()> {
        let storage = self.storage.clone();
        fut::wrap_future(self.storage.send(GetLogEntries{start: index, stop: index}))
            .map_err(|err, _: &mut Raft<S>, _| error!("{}{}", COMM_ERR_FETCH_LOG_ENTRIES, err))
            .and_then(|res, act, ctx| act.storage_engine_result(ctx, res))
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
                                .map_err(|err, _: &mut Raft<S>, _| error!("{}{}", COMM_ERR_FETCH_LOG_ENTRIES, err))
                                .and_then(|res, act, ctx| act.storage_engine_result(ctx, res))
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
    ) -> impl ActorFuture<Actor=Self, Item=proto::AppendEntriesResponse, Error=()> {
        // If message's term is less than most recent term, then we do not honor the request.
        let term = self.current_term;
        if msg.term < term {
            return fut::Either::A(
                fut::Either::A(fut::ok(proto::AppendEntriesResponse{term, success: false, conflict_opt: None}))
            );
        }

        // Update current term as needed & ensure we are in the follower state if needed.
        if msg.term > self.current_term {
            self.current_term = msg.term;
            self.state = NodeState::Follower;
        }

        // Kick off process of applying logs to state machine based on `msg.leader_commit`.
        self.commit_index = msg.leader_commit; // The value for `self.commit_index` is only updated here.
        if self.commit_index > self.last_applied && !self.is_applying_logs_to_state_machine {
            self.apply_logs_to_state_machine(ctx);
        }

        // If logs are already being appended, then just abort.
        if self.is_appending_logs {
            return fut::Either::A(
                fut::Either::A(fut::ok(proto::AppendEntriesResponse{term, success: false, conflict_opt: None}))
            );
        }

        // If RPC's `prev_log_index` is 0, or the RPC's previous log info matches the local
        // previous log info, then replication is g2g.
        if msg.prev_log_index == 0 || (msg.prev_log_index == self.last_log_index && msg.prev_log_term == self.last_log_term) {
            self.is_appending_logs = true;
            return fut::Either::A(fut::Either::B(
                fut::wrap_future(self.storage.send(AppendLogEntries(msg.entries)))
                    .map_err(|err, _: &mut Self, _| error!("{} {}", COMM_ERR_APPEND_LOG_ENTRIES, err))
                    .and_then(|res, act, ctx| act.storage_engine_result(ctx, res))
                    .map(move |res, act, _| {
                        act.last_log_index = res.index;
                        act.last_log_term = res.term;
                        proto::AppendEntriesResponse{term, success: true, conflict_opt: None}
                    })
                    .then(|res, act, _| {
                        act.is_appending_logs = false;
                        fut::FutureResult::from(res)
                    })));
        }

        // Previous log info doesn't immediately line up, so perform log consistency check and
        // proceed based on its result.
        let storage = self.storage.clone();
        self.is_appending_logs = true;
        fut::Either::B(self.log_consistency_check(ctx, msg.prev_log_index, msg.prev_log_term)
            .and_then(move |res, _, _| match res {
                Some(conflict_opt) => fut::Either::A(fut::FutureResult::from(Ok(
                    proto::AppendEntriesResponse{term, success: false, conflict_opt: Some(conflict_opt)}
                ))),
                None => fut::Either::B(fut::wrap_future(storage.send(AppendLogEntries(msg.entries)))
                    .map_err(|err, _: &mut Self, _| error!("{} {}", COMM_ERR_APPEND_LOG_ENTRIES, err))
                    .and_then(|res, act, ctx| act.storage_engine_result(ctx, res))
                    // At this point, we have successfully appended the new entries to the log.
                    // We now need to update our `last_log_*` info and build a response.
                    .and_then(move |res, act, _| {
                        act.last_log_index = res.index;
                        act.last_log_term = res.term;
                        fut::FutureResult::from(Ok(proto::AppendEntriesResponse{term, success: true, conflict_opt: None}))
                    })),
            })
            .then(|res, act, _| {
                act.is_appending_logs = false;
                fut::FutureResult::from(res)
            }))
    }

    /// Handle requests from peers to cast a vote for a new leader.
    fn handle_vote_request(&mut self, _ctx: &mut Context<Self>, msg: proto::VoteRequest) -> proto::VoteResponse {
        // If candidate's current term is less than this nodes current term, reject.
        if msg.term < self.current_term {
            return proto::VoteResponse{term: self.current_term, vote_granted: false};
        }

        // If candidate's log is not at least as up-to-date as this node, then reject.
        if msg.last_log_term < self.last_log_term || msg.last_log_index < self.last_log_index {
            return proto::VoteResponse{term: self.current_term, vote_granted: false};
        }

        // Candidate's log is as up-to-date, handle voting conditions.
        match &self.voted_for {
            // This node has already voted for the candidate.
            Some(candidate_id) if candidate_id == &msg.candidate_id => {
                proto::VoteResponse{term: self.current_term, vote_granted: true}
            }
            // This node has already voted for a different candidate.
            Some(_) => proto::VoteResponse{term: self.current_term, vote_granted: false},
            // This node has not already voted, so vote for the candidate.
            None => {
                self.voted_for = Some(msg.candidate_id);
                proto::VoteResponse{term: self.current_term, vote_granted: true}
            },
        }
    }

    /// Evaluate results coming from the storage engine and handle as needed.
    ///
    /// This routine is primarily used to simply stop this actor when an error has come up from
    /// the storage engine layer.
    fn storage_engine_result<T>(&mut self, ctx: &mut Context<Self>, res: Result<T, ()>) -> impl ActorFuture<Actor=Self, Item=T, Error=()> {
        fut::FutureResult::from(res.map_err(|_| {
            error!("Error from storage engine. Stopping Raft actor.");
            ctx.stop()
        }))
    }
}

impl<S: RaftStorage> Actor for Raft<S> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Fetch the node's initial state from the storage actor.
        let f = fut::wrap_future(self.storage.send(storage::GetInitialState))
            .map(|res: Result<InitialState, ()>, act: &mut Raft<S>, ctx| match res {
                Ok(state) => {
                    act.last_log_index = state.log_index;
                    act.last_log_term = state.log_term;
                    act.current_term = state.log_term;
                    act.voted_for = state.voted_for;
                    act.members = state.members;
                }
                Err(_) => {
                    error!("Error fetching initial state. Shutting down Raft actor.");
                    ctx.stop()
                }
            })

            // Fetch the node's last applied index from the storage actor.
            .and_then(|_, act, _| {
                fut::wrap_future(act.storage.send(storage::GetLastAppliedIndex))
            })
            .map(|res: Result<u64, ()>, act: &mut Raft<S>, ctx| match res {
                Ok(last_applied) => {
                    act.last_applied = last_applied;
                }
                Err(_) => {
                    error!("Error fetching last applied index. Shutting down Raft actor.");
                    ctx.stop()
                }
            })

            // Finish initialization if everything checks out.
            .map(|_, act, _| act.is_initialized = true)

            // Stop this actor if any messaging errors took place.
            .map_err(|_, _, ctx| {
                error!("Error communicating with storage layer. Shutting down Raft actor.");
                ctx.stop()
            });
        ctx.spawn(f);
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RaftRequest ///////////////////////////////////////////////////////////////////////////////////

/// An actix::Message wrapping a protobuf RaftRequest.
pub struct RaftRequest(pub proto::RaftRequest);

impl Message for RaftRequest {
    type Result = Result<proto::RaftResponse, ()>;
}

impl<S: RaftStorage> Handler<RaftRequest> for Raft<S> {
    type Result = ResponseActFuture<Self, proto::RaftResponse, ()>;

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
    ///
    /// TODO: maybe cut this over to use an actual error type to give users more control over how
    /// to handle errors. If we do so, shutting this actor down ourselves should not be required.
    fn handle(&mut self, msg: RaftRequest, ctx: &mut Self::Context) -> Self::Result {
        // Only handle requests if actor has finished initialization.
        if !self.is_initialized {
            warn!("Received RaftRequest before initialization was complete.");
            return Box::new(fut::err(()));
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
                Box::new(fut::ok(self.handle_vote_request(ctx, payload))
                    .map(|res, _, _| proto::RaftResponse{
                        payload: Some(ResponsePayload::Vote(res)),
                    }))
            },
            Some(Payload::InstallSnapshot(_payload)) => {
                // TODO: finish this up.
                Box::new(fut::err(()))
            },
            None => {
                warn!("RaftRequest received which had an empty or unknown payload.");
                Box::new(fut::err(()))
            }
        }
    }
}

// TODO:
// ### state
// - need to add GetInitialState.current_term. This means that initial state will mostly come from
// a "hard state" record which holds current_term, voted_for, and members.
// - need to persist hard state for things like current term (which may not be from most recent
// logs, but from RPCs). Add message type to storage interface. Call from needed location, which
// should pretty much always be due to an RPC.
// ### clients
// - create client message protobuf, used to generically wrap any type of client request.
//   Put together docs on how applications should mitigate client retries which would
//   lead to duplicates (request serial number tracking).
// - implement handler for client requests.
//
