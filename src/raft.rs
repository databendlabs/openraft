//! A module encapsulating the core `Raft` actor and its logic.

use std::collections::BTreeMap;

use actix::prelude::*;
use futures::future;

use crate::{
    messages::RaftMessage,
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
}

impl Raft {
    /// Create a new Raft instance.
    ///
    /// This actor will need to be started after instantiation, which must be done within a
    /// running actix system.
    ///
    /// TODO: add an example on how to create and start an instance.
    pub fn new(id: NodeId, members: Vec<NodeId>) -> Self {
        Self{id, members, commit_index: 0, last_applied: 0}
    }
}

impl Actor for Raft {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // TODO: Need to begin driving this actor.
    }
}

impl Handler<RaftMessage> for Raft {
    type Result = ResponseFuture<proto::RaftMessage, ()>;

    fn handle(&mut self, _msg: RaftMessage, _ctx: &mut Self::Context) -> Self::Result {
        // TODO:
        // - unpack the given message and pass to the appropriate handler.
        // - generate appropriate response message.
        // - create client message protobuf, used to generically wrap any type of client request.
        //   Put together docs on how applications should mitigate client retries which would
        //   lead to duplicates (request serial number tracking).
        // - implement handler for client requests.

        Box::new(future::err(()))
    }
}
