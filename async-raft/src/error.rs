//! Error types exposed by this crate.

use std::fmt;
use thiserror::Error;

use crate::{AppData, NodeId};

/// A result type where the error variant is always a `RaftError`.
pub type RaftResult<T> = std::result::Result<T, RaftError>;

/// Error variants related to the internals of Raft.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RaftError {
    /// An error which has come from the `RaftStorage` layer.
    #[error("{0}")]
    RaftStorage(anyhow::Error),
    /// An error which has come from the `RaftNetwork` layer.
    #[error("{0}")]
    RaftNetwork(anyhow::Error),
    /// An internal Raft error indicating that Raft is shutting down.
    #[error("Raft is shutting down")]
    ShuttingDown,
}

impl From<tokio::io::Error> for RaftError {
    fn from(src: tokio::io::Error) -> Self {
        RaftError::RaftStorage(src.into())
    }
}

/// An error related to a client read request.
#[derive(Debug, Error)]
pub enum ClientReadError {
    /// A Raft error.
    #[error("{0}")]
    RaftError(#[from] RaftError),
    /// The client read request must be forwarded to the cluster leader.
    #[error("the client read request must be forwarded to the cluster leader")]
    ForwardToLeader(Option<NodeId>),
}

/// An error related to a client write request.
#[derive(Error)]
pub enum ClientWriteError<D: AppData> {
    /// A Raft error.
    #[error("{0}")]
    RaftError(#[from] RaftError),
    /// The client write request must be forwarded to the cluster leader.
    #[error("the client write request must be forwarded to the cluster leader")]
    ForwardToLeader(D, Option<NodeId>),
}

impl<D: AppData> fmt::Debug for ClientWriteError<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientWriteError::RaftError(err) => f.debug_tuple("RaftError").field(err).finish(),
            ClientWriteError::ForwardToLeader(_req, node_id) => f.debug_tuple("ForwardToLeader").field(node_id).finish(),
        }
    }
}

/// Error variants related to configuration.
#[derive(Debug, Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConfigError {
    /// A configuration error indicating that the given values for election timeout min & max are invalid: max must be greater than min.
    #[error("given values for election timeout min & max are invalid: max must be greater than min")]
    InvalidElectionTimeoutMinMax,
    /// The given value for max_payload_entries is too small, must be > 0.
    #[error("the given value for max_payload_entries is too small, must be > 0")]
    MaxPayloadEntriesTooSmall,
}

/// The set of errors which may take place when initializing a pristine Raft node.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InitializeError {
    /// An internal error has taken place.
    #[error("{0}")]
    RaftError(#[from] RaftError),
    /// The requested action is not allowed due to the Raft node's current state.
    #[error("the requested action is not allowed due to the Raft node's current state")]
    NotAllowed,
}

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ChangeConfigError {
    /// An error related to the processing of the config change request.
    ///
    /// Errors of this type will only come about from the internals of applying the config change
    /// to the Raft log and the process related to that workflow.
    #[error("{0}")]
    RaftError(#[from] RaftError),
    /// The cluster is already undergoing a configuration change.
    #[error("the cluster is already undergoing a configuration change")]
    ConfigChangeInProgress,
    /// The given config would leave the cluster in an inoperable state.
    ///
    /// This error will be returned if the full set of changes, once fully applied, would leave
    /// the cluster in an inoperable state.
    #[error("the given config would leave the cluster in an inoperable state")]
    InoperableConfig,
    /// The node the config change proposal was sent to was not the leader of the cluster. The ID
    /// of the current leader is returned if known.
    #[error("this node is not the Raft leader")]
    NodeNotLeader(Option<NodeId>),
    /// The proposed config changes would make no difference to the current config.
    ///
    /// This takes into account a current joint consensus and the end result of the config.
    #[error("the proposed config change would have no effect, this is a no-op")]
    Noop,
}

impl<D: AppData> From<ClientWriteError<D>> for ChangeConfigError {
    fn from(src: ClientWriteError<D>) -> Self {
        match src {
            ClientWriteError::RaftError(err) => Self::RaftError(err),
            ClientWriteError::ForwardToLeader(_, id) => Self::NodeNotLeader(id),
        }
    }
}
