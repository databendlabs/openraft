//! Error types exposed by this crate.

use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;

use crate::raft::MembershipConfig;
use crate::raft_types::SnapshotSegmentId;
use crate::AppData;
use crate::LogId;
use crate::NodeId;
use crate::StorageError;

/// A result type where the error variant is always a `RaftError`.
pub type RaftResult<T> = std::result::Result<T, RaftError>;

/// Error variants related to the internals of Raft.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RaftError {
    // Streaming-snapshot encountered mismatched snapshot_id/offset
    #[error("expect: {expect}, got: {got}")]
    SnapshotMismatch {
        expect: SnapshotSegmentId,
        got: SnapshotSegmentId,
    },

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

/// Error variants related to the Replication.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum ReplicationError {
    #[error("seen a higher term: {higher} GT mine: {mine}")]
    HigherTerm { higher: u64, mine: u64 },

    #[error("Replication is closed")]
    Closed,

    #[error("{0}")]
    LackEntry(#[from] LackEntry),

    #[error("leader committed index {committed_index} advances target log index {target_index} too many")]
    CommittedAdvanceTooMany { committed_index: u64, target_index: u64 },

    // TODO(xp): two sub type: StorageError / TransportError
    // TODO(xp): a sub error for just send_append_entries()
    #[error("{0}")]
    StorageError(#[from] StorageError),

    #[error(transparent)]
    IO {
        #[backtrace]
        #[from]
        source: std::io::Error,
    },

    #[error("timeout after {timeout:?} to replicate {id}->{target}")]
    Timeout {
        id: NodeId,
        target: NodeId,
        timeout: Duration,
    },

    #[error(transparent)]
    Network {
        #[backtrace]
        source: anyhow::Error,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("store has no log at: {index}")]
pub struct LackEntry {
    pub index: u64,
}

impl From<tokio::io::Error> for RaftError {
    fn from(src: tokio::io::Error) -> Self {
        RaftError::RaftStorage(src.into())
    }
}

/// An error related to a client read request.
#[derive(Debug, thiserror::Error)]
pub enum ClientReadError {
    /// A Raft error.
    #[error("{0}")]
    RaftError(#[from] RaftError),
    /// The client read request must be forwarded to the cluster leader.
    #[error("the client read request must be forwarded to the cluster leader")]
    ForwardToLeader(Option<NodeId>),
}

/// An error related to a client write request.
#[derive(thiserror::Error)]
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
            ClientWriteError::ForwardToLeader(_req, node_id) => {
                f.debug_tuple("ForwardToLeader").field(node_id).finish()
            }
        }
    }
}

/// Error variants related to configuration.
#[derive(Debug, thiserror::Error, PartialEq)]
#[non_exhaustive]
pub enum ConfigError {
    /// A configuration error indicating that the given values for election timeout min & max are invalid: max must be
    /// greater than min.
    #[error("given values for election timeout min & max are invalid: max must be greater than min")]
    InvalidElectionTimeoutMinMax,

    /// The given value for max_payload_entries is too small, must be > 0.
    #[error("the given value for max_payload_entries is too small, must be > 0")]
    MaxPayloadEntriesTooSmall,

    /// election_timeout_min smaller than heartbeat_interval would cause endless election.
    /// A recommended election_timeout_min value is about 3 times heartbeat_interval.
    #[error("election_timeout_min value must be > heartbeat_interval")]
    ElectionTimeoutLessThanHeartBeatInterval,
}

/// The set of errors which may take place when initializing a pristine Raft node.
#[derive(Debug, thiserror::Error)]
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
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ChangeConfigError {
    /// An error related to the processing of the config change request.
    ///
    /// Errors of this type will only come about from the internals of applying the config change
    /// to the Raft log and the process related to that workflow.
    #[error("{0}")]
    RaftError(#[from] RaftError),

    /// The cluster is already undergoing a configuration change.
    #[error("the cluster is already undergoing a configuration change at log {membership_log_id}")]
    ConfigChangeInProgress { membership_log_id: LogId },

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

    // TODO(xp): 111 test it
    #[error("to add a member {node_id} first need to add it as non-voter")]
    NonVoterNotFound { node_id: NodeId },

    // TODO(xp): 111 test it
    #[error("replication to non voter {node_id} is lagging {distance}, can not add as member")]
    NonVoterIsLagging { node_id: NodeId, distance: u64 },

    // TODO(xp): test it in unittest
    // TOOO(xp): rename this error to some elaborated name.
    // TODO(xp): 111 test it
    #[error("now allowed to change from {curr:?} to {to:?}")]
    Incompatible {
        curr: MembershipConfig,
        to: BTreeSet<NodeId>,
    },

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

// A error wrapper of every type of error that will be sent to the caller.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResponseError {
    #[error(transparent)]
    ChangeConfig(#[from] ChangeConfigError),

    #[error(transparent)]
    Raft(#[from] RaftError),
}
