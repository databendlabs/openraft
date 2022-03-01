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
use crate::NodeId;
use crate::RPCTypes;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Vote;

/// Fatal is unrecoverable and shuts down raft at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum Fatal<NID: NodeId> {
    #[error(transparent)]
    StorageError(#[from] StorageError<NID>),

    #[error("raft stopped")]
    Stopped,
}

/// Extract Fatal from a Result.
///
/// Fatal will shutdown the raft and needs to be dealt separately,
/// such as StorageError.
pub trait ExtractFatal<NID: NodeId>
where Self: Sized
{
    fn extract_fatal(self) -> Result<Self, Fatal<NID>>;
}

impl<NID: NodeId, T, E> ExtractFatal<NID> for Result<T, E>
where E: TryInto<Fatal<NID>> + Clone
{
    fn extract_fatal(self) -> Result<Self, Fatal<NID>> {
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
pub enum AppendEntriesError<NID: NodeId> {
    #[error(transparent)]
    Fatal(#[from] Fatal<NID>),
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, derive_more::TryInto)]
pub enum VoteError<NID: NodeId> {
    #[error(transparent)]
    Fatal(#[from] Fatal<NID>),
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, derive_more::TryInto)]
pub enum InstallSnapshotError<NID: NodeId> {
    #[error(transparent)]
    SnapshotMismatch(#[from] SnapshotMismatch),

    #[error(transparent)]
    Fatal(#[from] Fatal<NID>),
}

/// An error related to a is_leader request.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, derive_more::TryInto)]
pub enum CheckIsLeaderError<C: RaftTypeConfig> {
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<NID>),

    #[error(transparent)]
    QuorumNotEnough(#[from] QuorumNotEnough<NID>),

    #[error(transparent)]
    Fatal(#[from] Fatal<NID>),
}

/// An error related to a client write request.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error, derive_more::TryInto)]
pub enum ClientWriteError<NID: NodeId> {
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<NID>),

    /// When writing a change-membership entry.
    #[error(transparent)]
    ChangeMembershipError(#[from] ChangeMembershipError<NID>),

    #[error(transparent)]
    Fatal(#[from] Fatal<NID>),
}

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum ChangeMembershipError<NID: NodeId> {
    #[error(transparent)]
    InProgress(#[from] InProgress<NID>),

    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    // TODO(xp): 111 test it
    #[error(transparent)]
    LearnerNotFound(#[from] LearnerNotFound<NID>),

    #[error(transparent)]
    LearnerIsLagging(#[from] LearnerIsLagging<NID>),

    #[error(transparent)]
    NodeNotInCluster(#[from] NodeIdNotInNodes<NID>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum AddLearnerError<NID: NodeId> {
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<NID>),

    #[error("node {0} is already a learner")]
    Exists(NID),

    #[error(transparent)]
    NodeNotInCluster(#[from] NodeIdNotInNodes<NID>),

    #[error(transparent)]
    Fatal(#[from] Fatal<NID>),
}

impl<NID: NodeId> TryFrom<AddLearnerError<NID>> for ForwardToLeader<NID> {
    type Error = AddLearnerError<NID>;

    fn try_from(value: AddLearnerError<NID>) -> Result<Self, Self::Error> {
        if let AddLearnerError::ForwardToLeader(e) = value {
            return Ok(e);
        }
        Err(value)
    }
}

/// The set of errors which may take place when initializing a pristine Raft node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum InitializeError<NID: NodeId> {
    /// The requested action is not allowed due to the Raft node's current state.
    #[error("the requested action is not allowed due to the Raft node's current state")]
    NotAllowed,

    #[error(transparent)]
    NodeNotInCluster(#[from] NodeIdNotInNodes<NID>),

    #[error(transparent)]
    Fatal(#[from] Fatal<NID>),
}

impl<NID: NodeId> From<StorageError<NID>> for AppendEntriesError<NID> {
    fn from(s: StorageError<NID>) -> Self {
        let f: Fatal<NID> = s.into();
        f.into()
    }
}
impl<NID: NodeId> From<StorageError<NID>> for VoteError<NID> {
    fn from(s: StorageError<NID>) -> Self {
        let f: Fatal<NID> = s.into();
        f.into()
    }
}
impl<NID: NodeId> From<StorageError<NID>> for InstallSnapshotError<NID> {
    fn from(s: StorageError<NID>) -> Self {
        let f: Fatal<NID> = s.into();
        f.into()
    }
}
impl<C: RaftTypeConfig> From<StorageError<C>> for CheckIsLeaderError<C> {
    fn from(s: StorageError<C>) -> Self {
        let f: Fatal<C> = s.into();
        f.into()
    }
}
impl<NID: NodeId> From<StorageError<NID>> for InitializeError<NID> {
    fn from(s: StorageError<NID>) -> Self {
        let f: Fatal<NID> = s.into();
        f.into()
    }
}
impl<NID: NodeId> From<StorageError<NID>> for AddLearnerError<NID> {
    fn from(s: StorageError<NID>) -> Self {
        let f: Fatal<NID> = s.into();
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
    pub last_purged_log_id: Option<LogId<C::NodeId>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("has to forward request to: {leader_id:?}, {leader_node:?}")]
pub struct ForwardToLeader<NID: NodeId> {
    pub leader_id: Option<NID>,
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
pub struct QuorumNotEnough<NID: NodeId> {
    pub cluster: String,
    pub got: BTreeSet<NID>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("the cluster is already undergoing a configuration change at log {membership_log_id}")]
pub struct InProgress<NID: NodeId> {
    pub membership_log_id: LogId<NID>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("to add a member {node_id} first need to add it as learner")]
pub struct LearnerNotFound<NID: NodeId> {
    pub node_id: NID,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("replication to learner {node_id} is lagging {distance}, matched: {matched:?}, can not add as member")]
pub struct LearnerIsLagging<NID: NodeId> {
    pub node_id: NID,
    pub matched: Option<LogId<NID>>,
    pub distance: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("node {node_id} not found in cluster: {node_ids:?}")]
pub struct NodeIdNotInNodes<NID: NodeId> {
    pub node_id: NID,
    pub node_ids: BTreeSet<NID>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("new membership can not be empty")]
pub struct EmptyMembership {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("node not found: {node_id}, source: {source}")]
pub struct NodeNotFound<NID: NodeId> {
    pub node_id: NID,
    pub source: AnyError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("infallible")]
pub enum Infallible {}
