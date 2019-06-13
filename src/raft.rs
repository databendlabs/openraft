//! A module encapsulating the core `Raft` actor and its logic.

use std::collections::BTreeMap;

use actix::prelude::*;
use futures::future;
use log::{warn};

use crate::{
    messages::RaftRequest,
    proto,
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
pub struct Raft {
    /// This node's ID.
    id: NodeId,
    /// All currently known members of the Raft cluster.
    members: Vec<NodeId>,
    /// The index of the highest log entry known to be committed.
    ///
    /// Is initialized to 0, and increases monotonically.
    commit_index: u64,
    /// The index of the highest log entry which has been applied to the state machine.
    ///
    /// Is initialized to 0, increases monotonically following the `commit_index`.
    last_applied: u64,
    /// The current state of this Raft node.
    state: NodeState,
    /// An output channel for sending Raft request messages to peers.
    out: actix::Recipient<RaftRequest>,
}

impl Raft {
    /// Create a new Raft instance.
    ///
    /// This actor will need to be started after instantiation, which must be done within a
    /// running actix system.
    ///
    /// TODO: add an example on how to create and start an instance.
    pub fn new(id: NodeId, members: Vec<NodeId>, out: actix::Recipient<RaftRequest>) -> Self {
        let state = NodeState::Follower;
        Self{id, members, commit_index: 0, last_applied: 0, state, out}
    }
}

impl Actor for Raft {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // TODO: Need to begin driving this actor.
    }
}

impl Handler<RaftRequest> for Raft {
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
    fn handle(&mut self, msg: RaftRequest, _ctx: &mut Self::Context) -> Self::Result {
        // Unpack the given message and pass to the appropriate handler.
        use proto::raft_request::Payload;
        match msg.0.payload {
            Some(Payload::AppendEntries(_payload)) => (),
            Some(Payload::Vote(_payload)) => (),
            Some(Payload::InstallSnapshot(_payload)) => (),
            None => warn!("RaftRequest received which had an empty or unknown payload."),
        };

        // TODO:
        // - create client message protobuf, used to generically wrap any type of client request.
        //   Put together docs on how applications should mitigate client retries which would
        //   lead to duplicates (request serial number tracking).
        // - implement handler for client requests.

        Box::new(future::err(()))
    }
}
