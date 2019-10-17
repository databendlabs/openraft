//! Admin message types used to initialize & control a Raft node.

use actix::prelude::*;

use crate::{
    AppData, AppDataResponse, AppError, NodeId,
    messages::ClientError,
};

/// Initialize a pristine Raft node with the given config & start a campaign to become leader.
pub struct InitWithConfig {
    /// All currently known members to initialize the new cluster with.
    ///
    /// If the ID of the node this command is being submitted to is not present it will be added.
    /// If there are duplicates, they will be filtered out to ensure config is proper.
    pub members: Vec<NodeId>,
}

impl InitWithConfig {
    /// Construct a new instance.
    pub fn new(members: Vec<NodeId>) -> Self {
        Self{members}
    }
}

impl Message for InitWithConfig {
    type Result = Result<(), InitWithConfigError>;
}

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug)]
pub enum InitWithConfigError {
    /// An internal error has taken place.
    Internal,
    /// Submission of this command to this node is not allowed due to the state of the node.
    NotAllowed,
}

impl std::fmt::Display for InitWithConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitWithConfigError::Internal => write!(f, "An error internal to Raft has taken place."),
            InitWithConfigError::NotAllowed => write!(f, "Submission of this command to this node is not allowed due to the state of the node."),
        }
    }
}

impl std::error::Error for InitWithConfigError {}

//////////////////////////////////////////////////////////////////////////////////////////////////
// ProposeConfigChange ///////////////////////////////////////////////////////////////////////////

/// Propose a new membership config change to a running cluster.
///
/// There are a few invariants which must be upheld here:
///
/// - if the node this command is sent to is not the leader of the cluster, it will be rejected.
/// - if the given changes would leave the cluster in an inoperable state, it will be rejected.
pub struct ProposeConfigChange<D: AppData, R: AppDataResponse, E: AppError> {
    /// New members to be added to the cluster.
    pub(crate) add_members: Vec<NodeId>,
    /// Members to be removed from the cluster.
    pub(crate) remove_members: Vec<NodeId>,
    marker_data: std::marker::PhantomData<D>,
    marker_res: std::marker::PhantomData<R>,
    marker_error: std::marker::PhantomData<E>,
}

impl<D: AppData, R: AppDataResponse, E: AppError> ProposeConfigChange<D, R, E> {
    /// Create a new instance.
    ///
    /// If there are duplicates in either of the givenn vectors, they will be filtered out to
    /// ensure config is proper.
    pub fn new(add_members: Vec<NodeId>, remove_members: Vec<NodeId>) -> Self {
        Self{add_members, remove_members, marker_data: std::marker::PhantomData, marker_res: std::marker::PhantomData, marker_error: std::marker::PhantomData}
    }
}

impl<D: AppData, R: AppDataResponse, E: AppError> Message for ProposeConfigChange<D, R, E> {
    type Result = Result<(), ProposeConfigChangeError<D, R, E>>;
}

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug)]
pub enum ProposeConfigChangeError<D: AppData, R: AppDataResponse, E: AppError> {
    /// An error related to the processing of the config change request.
    ///
    /// Errors of this type will only come about from the internals of applying the config change
    /// to the Raft log and the process related to that workflow.
    ClientError(ClientError<D, R, E>),
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

impl<D: AppData, R: AppDataResponse, E: AppError> std::fmt::Display for ProposeConfigChangeError<D, R, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProposeConfigChangeError::ClientError(err) => write!(f, "{}", err),
            ProposeConfigChangeError::InoperableConfig => write!(f, "The given config would leave the cluster in an inoperable state."),
            ProposeConfigChangeError::Internal => write!(f, "An error internal to Raft has taken place."),
            ProposeConfigChangeError::NodeNotLeader(leader_opt) => write!(f, "The handling node is not the Raft leader. Tracked value for cluster leader: {:?}", leader_opt),
            ProposeConfigChangeError::Noop => write!(f, "The proposed config change would have no effect, this is a no-op."),
        }
    }
}

impl<D: AppData, R: AppDataResponse, E: AppError> std::error::Error for ProposeConfigChangeError<D, R, E> {}
