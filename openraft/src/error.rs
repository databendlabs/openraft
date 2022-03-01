//! Error types exposed by this crate.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Debug;
use std::time::Duration;

use anyerror::AnyError;
use serde::Deserialize;
use serde::Serialize;

use crate::raft_types::SnapshotSegmentId;
use crate::LogId;
use crate::Node;
use crate::RPCTypes;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Vote;

/// Fatal is unrecoverable and shuts down raft at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum Fatal<C: RaftTypeConfig> {
    #[error(transparent)]
    StorageError(#[from] StorageError<C>),

    #[error("raft stopped")]
    Stopped,
}

/// Extract Fatal from a Result.
///
/// Fatal will shutdown the raft and needs to be dealt separately,
/// such as StorageError.
pub trait ExtractFatal<C: RaftTypeConfig>
where Self: Sized
{
    fn extract_fatal(self) -> Result<Self, Fatal<C>>;
}

impl<C: RaftTypeConfig, T, E> ExtractFatal<C> for Result<T, E>
where E: TryInto<Fatal<C>> + Clone
{
    fn extract_fatal(self) -> Result<Self, Fatal<C>> {
        if let Err(e) = &self {
            let fatal = e.clone().try_into();
            if let Ok(f) = fatal {
                return Err(f);
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, derive_more::TryInto)]
pub enum AppendEntriesError<C: RaftTypeConfig> {
    #[error(transparent)]
    Fatal(#[from] Fatal<C>),
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, derive_more::TryInto)]
pub enum VoteError<C: RaftTypeConfig> {
    #[error(transparent)]
    Fatal(#[from] Fatal<C>),
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, derive_more::TryInto)]
pub enum InstallSnapshotError<C: RaftTypeConfig> {
    #[error(transparent)]
    SnapshotMismatch(#[from] SnapshotMismatch),

    #[error(transparent)]
    Fatal(#[from] Fatal<C>),
}

/// An error related to a client read request.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, derive_more::TryInto)]
pub enum ClientReadError<C: RaftTypeConfig> {
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<C>),

    #[error(transparent)]
    QuorumNotEnough(#[from] QuorumNotEnough<C>),

    #[error(transparent)]
    Fatal(#[from] Fatal<C>),
}

/// An error related to a client write request.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, derive_more::TryInto)]
pub enum ClientWriteError<C: RaftTypeConfig> {
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<C>),

    /// When writing a change-membership entry.
    #[error(transparent)]
    ChangeMembershipError(#[from] ChangeMembershipError<C>),

    #[error(transparent)]
    Fatal(#[from] Fatal<C>),
}

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum ChangeMembershipError<C: RaftTypeConfig> {
    #[error(transparent)]
    InProgress(#[from] InProgress<C>),

    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    // TODO(xp): 111 test it
    #[error(transparent)]
    LearnerNotFound(#[from] LearnerNotFound<C>),

    #[error(transparent)]
    LearnerIsLagging(#[from] LearnerIsLagging<C>),

    #[error(transparent)]
    NodeNotInCluster(#[from] NodeIdNotInNodes<C>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum AddLearnerError<C: RaftTypeConfig> {
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<C>),

    #[error("node {0} is already a learner")]
    Exists(C::NodeId),

    #[error(transparent)]
    NodeNotInCluster(#[from] NodeIdNotInNodes<C>),

    #[error(transparent)]
    Fatal(#[from] Fatal<C>),
}

impl<C: RaftTypeConfig> TryFrom<AddLearnerError<C>> for ForwardToLeader<C> {
    type Error = AddLearnerError<C>;

    fn try_from(value: AddLearnerError<C>) -> Result<Self, Self::Error> {
        if let AddLearnerError::ForwardToLeader(e) = value {
            return Ok(e);
        }
        Err(value)
    }
}

/// The set of errors which may take place when initializing a pristine Raft node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum InitializeError<C: RaftTypeConfig> {
    /// The requested action is not allowed due to the Raft node's current state.
    #[error("the requested action is not allowed due to the Raft node's current state")]
    NotAllowed,

    #[error(transparent)]
    NodeNotInCluster(#[from] NodeIdNotInNodes<C>),

    #[error(transparent)]
    Fatal(#[from] Fatal<C>),
}

impl<C: RaftTypeConfig> From<StorageError<C>> for AppendEntriesError<C> {
    fn from(s: StorageError<C>) -> Self {
        let f: Fatal<C> = s.into();
        f.into()
    }
}
impl<C: RaftTypeConfig> From<StorageError<C>> for VoteError<C> {
    fn from(s: StorageError<C>) -> Self {
        let f: Fatal<C> = s.into();
        f.into()
    }
}
impl<C: RaftTypeConfig> From<StorageError<C>> for InstallSnapshotError<C> {
    fn from(s: StorageError<C>) -> Self {
        let f: Fatal<C> = s.into();
        f.into()
    }
}
impl<C: RaftTypeConfig> From<StorageError<C>> for ClientReadError<C> {
    fn from(s: StorageError<C>) -> Self {
        let f: Fatal<C> = s.into();
        f.into()
    }
}
impl<C: RaftTypeConfig> From<StorageError<C>> for InitializeError<C> {
    fn from(s: StorageError<C>) -> Self {
        let f: Fatal<C> = s.into();
        f.into()
    }
}
impl<C: RaftTypeConfig> From<StorageError<C>> for AddLearnerError<C> {
    fn from(s: StorageError<C>) -> Self {
        let f: Fatal<C> = s.into();
        f.into()
    }
}

/// Error variants related to the Replication.
#[derive(Debug, thiserror::Error)]
#[allow(clippy::large_enum_variant)]
pub enum ReplicationError<C: RaftTypeConfig> {
    #[error(transparent)]
    HigherVote(#[from] HigherVote<C>),

    #[error("Replication is closed")]
    Closed,

    #[error(transparent)]
    LackEntry(#[from] LackEntry<C>),

    #[error(transparent)]
    CommittedAdvanceTooMany(#[from] CommittedAdvanceTooMany),

    // TODO(xp): two sub type: StorageError / TransportError
    // TODO(xp): a sub error for just send_append_entries()
    #[error(transparent)]
    StorageError(#[from] StorageError<C>),

    #[error(transparent)]
    NodeNotFound(#[from] NodeNotFound<C>),

    #[error(transparent)]
    Timeout(#[from] Timeout<C>),

    #[error(transparent)]
    Network(#[from] NetworkError),

    #[error(transparent)]
    RemoteError(#[from] RemoteError<C, AppendEntriesError<C>>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum RPCError<C: RaftTypeConfig, T: Error> {
    #[error(transparent)]
    NodeNotFound(#[from] NodeNotFound<C>),

    #[error(transparent)]
    Timeout(#[from] Timeout<C>),

    #[error(transparent)]
    Network(#[from] NetworkError),

    #[error(transparent)]
    RemoteError(#[from] RemoteError<C, T>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("error occur on remote peer {target}: {source}")]
pub struct RemoteError<C: RaftTypeConfig, T: std::error::Error> {
    pub target: C::NodeId,
    pub target_node: Option<Node>,
    pub source: T,
}

impl<C: RaftTypeConfig, T: std::error::Error> RemoteError<C, T> {
    pub fn new(target: C::NodeId, e: T) -> Self {
        Self {
            target,
            target_node: None,
            source: e,
        }
    }
    pub fn new_with_node(target: C::NodeId, node: Node, e: T) -> Self {
        Self {
            target,
            target_node: Some(node),
            source: e,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("seen a higher vote: {higher} GT mine: {mine}")]
pub struct HigherVote<C: RaftTypeConfig> {
    pub higher: Vote<C>,
    pub mine: Vote<C>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("leader committed index {committed_index} advances target log index {target_index} too many")]
pub struct CommittedAdvanceTooMany {
    pub committed_index: u64,
    pub target_index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error(transparent)]
pub struct NetworkError {
    #[from]
    source: AnyError,
}

impl NetworkError {
    pub fn new<E: Error + 'static>(e: &E) -> Self {
        Self {
            source: AnyError::new(e),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("timeout after {timeout:?} when {action} {id}->{target}")]
pub struct Timeout<C: RaftTypeConfig> {
    pub action: RPCTypes,
    pub id: C::NodeId,
    pub target: C::NodeId,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("store has no log at: {index:?}, last purged: {last_purged_log_id:?}")]
pub struct LackEntry<C: RaftTypeConfig> {
    pub index: Option<u64>,
    pub last_purged_log_id: Option<LogId<C>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("has to forward request to: {leader_id:?}, {leader_node:?}")]
pub struct ForwardToLeader<C: RaftTypeConfig> {
    pub leader_id: Option<C::NodeId>,
    pub leader_node: Option<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("snapshot segment id mismatch, expect: {expect}, got: {got}")]
pub struct SnapshotMismatch {
    pub expect: SnapshotSegmentId,
    pub got: SnapshotSegmentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("not enough for a quorum, cluster: {cluster}, got: {got:?}")]
pub struct QuorumNotEnough<C: RaftTypeConfig> {
    pub cluster: String,
    pub got: BTreeSet<C::NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("the cluster is already undergoing a configuration change at log {membership_log_id}")]
pub struct InProgress<C: RaftTypeConfig> {
    pub membership_log_id: LogId<C>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("to add a member {node_id} first need to add it as learner")]
pub struct LearnerNotFound<C: RaftTypeConfig> {
    pub node_id: C::NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("replication to learner {node_id} is lagging {distance}, matched: {matched:?}, can not add as member")]
pub struct LearnerIsLagging<C: RaftTypeConfig> {
    pub node_id: C::NodeId,
    pub matched: Option<LogId<C>>,
    pub distance: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("node {node_id} not found in cluster: {node_ids:?}")]
pub struct NodeIdNotInNodes<C: RaftTypeConfig> {
    pub node_id: C::NodeId,
    pub node_ids: BTreeSet<C::NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("new membership can not be empty")]
pub struct EmptyMembership {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("node not found: {node_id}, source: {source}")]
pub struct NodeNotFound<C: RaftTypeConfig> {
    pub node_id: C::NodeId,
    pub source: AnyError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("infallible")]
pub enum Infallible {}
