//! Error types exposed by this crate.

use thiserror::Error;

use crate::{AppData, AppError, NodeId};
use crate::raft::ClientRequest;

/// A result type where the error variant is always a `RaftError`.
pub type RaftResult<T, E> = std::result::Result<T, RaftError<E>>;

/// Error variants related to the internals of Raft.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RaftError<E: AppError> {
    /// An application error.
    #[error("{0}")]
    AppError(#[from] E),
    /// A configuration error indicating that the given values for election timeout min & max are invalid: max must be greater than min.
    #[error("given values for election timeout min & max are invalid: max must be greater than min")]
    InvalidElectionTimeoutMinMax,
    /// An internal Raft error indicating that Raft is shutting down.
    #[error("Raft is shutting down")]
    ShuttingDown,
    /// An IO error from tokio.
    #[error("{0}")]
    IO(#[from] tokio::io::Error)
}

/// An error related to a client request.
#[derive(Debug, Error)]
pub enum ClientError<D: AppData, E: AppError> {
    /// A Raft error.
    #[error("{0}")]
    RaftError(#[from] RaftError<E>),
    /// The client request must be forwarded to the cluster leader.
    #[error("the client request must be forwarded to the cluster leader")]
    ForwardToLeader(ClientRequest<D>, Option<NodeId>),
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

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug, Error)]
pub enum InitWithConfigError<E: AppError> {
    /// An internal error has taken place.
    #[error("{0}")]
    RaftError(#[from] RaftError<E>),
    /// The requested action is not allowed due to the Raft node's current state.
    #[error("the requested action is not allowed due to the Raft node's current state")]
    NotAllowed,
}

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug, Error)]
pub enum ProposeConfigChangeError<E: AppError> {
    /// An error related to the processing of the config change request.
    ///
    /// Errors of this type will only come about from the internals of applying the config change
    /// to the Raft log and the process related to that workflow.
    #[error("{0}")]
    RaftError(#[from] RaftError<E>),
    /// The cluster is already undergoing a configuration change.
    #[error("the cluster is already undergoing a configuration change")]
    AlreadyInJointConsensus,
    /// The given config would leave the cluster in an inoperable state.
    ///
    /// This error will be returned if the full set of changes, once fully applied, would leave
    /// the cluster with less than two members.
    #[error("the given config would leave the cluster in an inoperable state")]
    InoperableConfig,
    /// The node the config change proposal was sent to was not the leader of the cluster.
    #[error("this node is not the Raft leader")]
    NodeNotLeader,
    /// The proposed config changes would make no difference to the current config.
    ///
    /// This takes into account a current joint consensus and the end result of the config.
    ///
    /// This error will be returned if the proposed add & remove elements are empty; all of the
    /// entries to be added already exist in the current config and/or all of the entries to be
    /// removed have already been scheduled for removal and/or do not exist in the current config.
    #[error("the proposed config change would have no effect, this is a no-op")]
    Noop,
}

impl<D: AppData, E: AppError> From<ClientError<D, E>> for ProposeConfigChangeError<E> {
    fn from(src: ClientError<D, E>) -> Self {
        match src {
            ClientError::RaftError(err) => Self::RaftError(err),
            ClientError::ForwardToLeader(_, _) => Self::NodeNotLeader,
        }
    }
}
