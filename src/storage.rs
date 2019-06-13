//! A module encapsulating the Raft storage interface.

use actix::{
    dev::ToEnvelope,
    prelude::*,
};

use crate::raft::NodeId;

/// A struct used to represent the initial state which a Raft node needs when first starting.
pub struct InitialState {
    pub log_index: u64,
    pub log_term: u64,
    pub voted_for: Option<NodeId>,
}

/// An actix message type for requesting Raft state information from the storage layer.
pub struct GetInitialState;

impl Message for GetInitialState {
    type Result = Result<InitialState, ()>;
}

/// An actix message type for requesting the index of the last log applied to the state machine.
pub struct GetLastAppliedIndex;

impl Message for GetLastAppliedIndex {
    type Result = Result<u64, ()>;
}

/// A trait describing the interface of a Raft storage actor.
pub trait RaftStorage
    where
        Self: Actor<Context=Context<Self>>,
        Self: Handler<GetInitialState> + ToEnvelope<Self, GetInitialState>,
        Self: Handler<GetLastAppliedIndex> + ToEnvelope<Self, GetLastAppliedIndex>,
{}
