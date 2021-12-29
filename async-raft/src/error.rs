//! Error types exposed by this crate.

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::time::Duration;

use crate::raft::Membership;
use crate::raft_types::SnapshotSegmentId;
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

#[derive(Debug, thiserror::Error)]
#[error("has to forward request to: {leader_id:?}")]
pub struct ForwardToLeader {
    pub leader_id: Option<NodeId>,
}

impl From<tokio::io::Error> for RaftError {
    fn from(src: tokio::io::Error) -> Self {
        RaftError::RaftStorage(src.into())
    }
}

/// An error related to a client read request.
#[derive(Debug, thiserror::Error)]
pub enum ClientReadError {
    #[error(transparent)]
    RaftError(#[from] RaftError),

    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader),
}

/// An error related to a client write request.
#[derive(thiserror::Error, Debug, derive_more::TryInto)]
pub enum ClientWriteError {
    #[error("{0}")]
    RaftError(#[from] RaftError),

    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader),

    /// When writing a change-membership entry.
    #[error(transparent)]
    ChangeMembershipError(#[from] ChangeMembershipError),
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
pub enum ChangeMembershipError {
    #[error("the cluster is already undergoing a configuration change at log {membership_log_id}")]
    InProgress { membership_log_id: LogId },

    #[error("new membership can not be empty")]
    EmptyMembership,

    // TODO(xp): 111 test it
    #[error("to add a member {node_id} first need to add it as non-voter")]
    NonVoterNotFound { node_id: NodeId },

    // TODO(xp): 111 test it
    #[error("replication to non voter {node_id} is lagging {distance}, matched: {matched}, can not add as member")]
    NonVoterIsLagging {
        node_id: NodeId,
        matched: LogId,
        distance: u64,
    },

    // TODO(xp): test it in unittest
    // TODO(xp): rename this error to some elaborated name.
    // TODO(xp): 111 test it
    #[error("now allowed to change from {curr:?} to {to:?}")]
    Incompatible { curr: Membership, to: BTreeSet<NodeId> },
}

#[derive(Debug, thiserror::Error)]
pub enum AddNonVoterError {
    #[error("{0}")]
    RaftError(#[from] RaftError),

    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader),

    #[error("node {0} is already a non-voter")]
    Exists(NodeId),
}
