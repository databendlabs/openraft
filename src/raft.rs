//! A module encapsulating the core `Raft` actor and its logic.

use std::collections::BTreeMap;

use actix::prelude::*;

/// The state of the Raft node.
pub enum NodeState {
    Learner,
    Candidate,
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
    next_index: BTreeMap<u64, u64>,
    /// A mapping of node IDs to the highest log entry index known to be replicated thereof.
    ///
    /// Each entry is initialized to 0, and increases per the Raft data replication protocol.
    match_index: BTreeMap<u64, u64>,
}

/// An actor which implements the Raft protocol's core business logic.
///
/// For more information on the Raft protocol, see the specification here:
/// https://raft.github.io/raft.pdf (**pdf warning**).
///
/// The beginning of §5 in the spec has a condensed summary of the Raft consensus algorithm. This
/// crate, and especially this actor, attempts to follow the terminology and nomenclature used
/// there as precisely as possible to aid in understanding this system.
pub struct Raft {
    /// The index of the highest log entry known to becommitted.
    ///
    /// Is initialized to 0, and increases monotonically.
    commit_index: u64,
    /// The index of the highest log entry which has been applied to the statemachine.
    ///
    /// Is initialized to 0, increases monotonically following the `commit_index`.
    last_applied: u64,
}
