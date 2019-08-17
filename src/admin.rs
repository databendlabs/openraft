//! Admin message types used to initialize & control a Raft node.

use actix::prelude::*;

use crate::{
    AppError, NodeId,
    messages::ClientError,
};

/// Initialize a pristine Raft node with the given config & start a campaign to become leader.
pub struct InitWithConfig;

/// Propose a new config change to a running cluster.
///
/// There are a few invariants which must be upheld here:
///
/// - if the node this command is sent to is not the leader of the cluster, it will be rejected.
/// - if the given changes would leave the cluster in an inoperable state, it will be rejected.
pub struct ProposeConfigChange<E: AppError> {
    /// New members to be added to the cluster.
    pub(crate) add_members: Vec<NodeId>,
    /// Members to be removed from the cluster.
    pub(crate) remove_members: Vec<NodeId>,
    marker: std::marker::PhantomData<E>,
}

impl<E: AppError> ProposeConfigChange<E> {
    /// Create a new instance.
    pub fn new(add_members: Vec<NodeId>, remove_members: Vec<NodeId>) -> Self {
        Self{add_members, remove_members, marker: std::marker::PhantomData}
    }
}

impl<E: AppError> Message for ProposeConfigChange<E> {
    type Result = Result<(), ProposeConfigChangeError<E>>;
}

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug)]
pub enum ProposeConfigChangeError<E: AppError> {
    /// An error related to the processing of the config change request.
    ///
    /// Errors of this type will only come about from the internals of applying the config change
    /// to the Raft log and the process related to that workflow.
    ClientError(ClientError<E>),
    /// The given config would leave the cluster in an inoperable state.
    ///
    /// This error will be returned if the full set of changes, once fully applied, would leave
    /// the cluster with less than two members.
    InoperableConfig,
    /// An internal error has taken place.
    ///
    /// These should never normally take place, but if one is encountered, it should be safe to
    /// retry the operation.
    Internal,
    /// The node the config change proposal was sent to was not the leader of the cluster.
    ///
    /// If the current cluster leader is known, its ID will be wrapped in this variant.
    NodeNotLeader(Option<NodeId>),
    /// The proposed config changes would make no difference to the current config.
    ///
    /// This takes into account a current joint consensus and the end result of the config.
    ///
    /// This error will be returned if the proposed add & remove elements are empty; all of the
    /// entries to be added already exist in the current config and/or all of the entries to be
    /// removed have already been scheduled for removal and/or do not exist in the current config.
    Noop,
}
