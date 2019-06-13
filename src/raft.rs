//! A module encapsulating the core `Raft` actor and its logic.

use std::collections::BTreeMap;

use actix::prelude::*;
use futures::future;
use log::{error, warn};

use crate::{
    proto,
    storage::{self, InitialState, RaftStorage},
};

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
/// This interface is still in the works. Hopefully something like:
/// `actix::Adder<T> where T: actix::Handler<StorageMsg0> + actix::Handler<StorageMsg1> ...`.
///
/// TODO: finish this interface up.
pub struct Raft<S: RaftStorage> {
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
    /// The index of the highest log entry known to be committed.
    ///
    /// Is initialized to 0, and increases monotonically.
    commit_index: u64,
    /// The latest term this node has seen.
    ///
    /// Is initialized to 0 on first boot, and increases monotonically.
    current_term: u64,
    /// The ID of the candidate which received this node's vote for the current term.
    ///
    /// Each server will vote for at most one candidate in a given term, on a
    /// first-come-first-served basis. See §5.4.1 for additional restriction on votes.
    voted_for: Option<NodeId>,
    /// The index of the highest log entry which has been applied to the state machine.
    ///
    /// Is initialized to 0, increases monotonically following the `commit_index` as logs are
    /// applied to the state machine (via the storage interface).
    last_applied: u64,
    /// A value indicating if this actor has successfully completed its initialization routine.
    is_initialized: bool,
}

impl<S> Raft<S> where S: RaftStorage {
    /// Create a new Raft instance.
    ///
    /// This actor will need to be started after instantiation, which must be done within a
    /// running actix system.
    ///
    /// TODO: add an example on how to create and start an instance.
    pub fn new(id: NodeId, members: Vec<NodeId>, out: actix::Recipient<RaftRequest>, storage: Addr<S>) -> Self {
        let state = NodeState::Follower;
        Self{
            id, members, state, out, storage,
            commit_index: 0, current_term: 0, voted_for: None, last_applied: 0,
            is_initialized: false,
        }
    }

    /// Handle requests from peers to cast a vote for a new leader.
    fn handle_vote_request(&mut self, _ctx: &mut Context<Self>, msg: proto::VoteRequest) -> proto::VoteResponse {
        // If candidate's log is not as least as up-to-date as this node, then reject.
        if msg.last_log_term < self.current_term || msg.last_log_index < self.commit_index {
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
}

impl<S: RaftStorage> Actor for Raft<S> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Fetch the node's initial state from the storage actor.
        let f = fut::wrap_future(self.storage.send(storage::GetInitialState))
            .map(|res: Result<InitialState, ()>, act: &mut Raft<S>, ctx| match res {
                Ok(state) => {
                    act.commit_index = state.log_index;
                    act.current_term = state.log_term;
                    act.voted_for = state.voted_for;
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
    type Result = ResponseFuture<proto::RaftResponse, ()>;

    /// Handle inbound Raft request messages.
    ///
    /// Typically the Raft messages sent to this handler will have come from peer nodes of the
    /// Raft cluster, usuall via your application's networking layer. This handler guarantees that
    /// the appropriate Raft response message type will be returned in response according to the
    /// Raft spec.
    ///
    /// ### errors
    /// It is rare that an error will be returned from this handler. The most typical error
    /// conditions are related to storage failures. In the case of a storage failure however,
    /// this actor will typically shut down, so this interface may end up not having an error
    /// case.
    ///
    /// TODO: pin down error cases and update docs on how storage errors are handled.
    fn handle(&mut self, msg: RaftRequest, ctx: &mut Self::Context) -> Self::Result {
        // Unpack the given message and pass to the appropriate handler.
        use proto::raft_request::Payload;
        use proto::raft_response::Payload as ResponsePayload;
        match msg.0.payload {
            Some(Payload::AppendEntries(_payload)) => {
                // TODO: finish this up.
                Box::new(future::err(()))
            },
            Some(Payload::Vote(payload)) => {
                let res = self.handle_vote_request(ctx, payload);
                Box::new(future::ok(proto::RaftResponse{
                    payload: Some(ResponsePayload::Vote(res)),
                }))
            },
            Some(Payload::InstallSnapshot(_payload)) => {
                // TODO: finish this up.
                Box::new(future::err(()))
            },
            None => {
                warn!("RaftRequest received which had an empty or unknown payload.");
                Box::new(future::err(()))
            }
        }
    }
}

// TODO:
// ### clients
// - create client message protobuf, used to generically wrap any type of client request.
//   Put together docs on how applications should mitigate client retries which would
//   lead to duplicates (request serial number tracking).
// - implement handler for client requests.
//
// ### config
// - build config struct. Needs:
//   - election timeout min
//   - election timeout max
//
// ### storage
// - GetInitialState. Must return highest log index, highest log term, node voted for in highest term.
// - GetLastAppliedIndex. Must return the index of the highest log entry applied to the state machine.
