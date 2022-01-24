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
use crate::NodeId;
use crate::RPCTypes;
use crate::StorageError;

/// Fatal is unrecoverable and shuts down raft at once.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fatal {
    #[error(transparent)]
    StorageError(#[from] StorageError),

    #[error("raft stopped")]
    Stopped,
}

/// Extract Fatal from a Result.
///
/// Fatal will shutdown the raft and needs to be dealt separately,
/// such as StorageError.
pub trait ExtractFatal
where Self: Sized
{
    fn extract_fatal(self) -> Result<Self, Fatal>;
}

impl<T, E> ExtractFatal for Result<T, E>
where E: TryInto<Fatal> + Clone
{
    fn extract_fatal(self) -> Result<Self, Fatal> {
        if let Err(e) = &self {
            let fatal = e.clone().try_into();
            if let Ok(f) = fatal {
                return Err(f);
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
pub enum AppendEntriesError {
    #[error(transparent)]
    Fatal(#[from] Fatal),
}

#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
pub enum VoteError {
    #[error(transparent)]
    Fatal(#[from] Fatal),
}

#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
pub enum InstallSnapshotError {
    #[error(transparent)]
    SnapshotMismatch(#[from] SnapshotMismatch),

    #[error(transparent)]
    Fatal(#[from] Fatal),
}

/// An error related to a client read request.
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
pub enum ClientReadError {
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader),

    #[error(transparent)]
    QuorumNotEnough(#[from] QuorumNotEnough),

    #[error(transparent)]
    Fatal(#[from] Fatal),
}

/// An error related to a client write request.
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
pub enum ClientWriteError {
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader),

    /// When writing a change-membership entry.
    #[error(transparent)]
    ChangeMembershipError(#[from] ChangeMembershipError),

    #[error(transparent)]
    Fatal(#[from] Fatal),
}

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum ChangeMembershipError {
    #[error(transparent)]
    InProgress(#[from] InProgress),

    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    // TODO(xp): 111 test it
    #[error(transparent)]
    LearnerNotFound(#[from] LearnerNotFound),

    #[error(transparent)]
    LearnerIsLagging(#[from] LearnerIsLagging),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum AddLearnerError {
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader),

    #[error("node {0} is already a learner")]
    Exists(NodeId),

    #[error(transparent)]
    Fatal(#[from] Fatal),
}

/// The set of errors which may take place when initializing a pristine Raft node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum InitializeError {
    /// The requested action is not allowed due to the Raft node's current state.
    #[error("the requested action is not allowed due to the Raft node's current state")]
    NotAllowed,

    #[error(transparent)]
    Fatal(#[from] Fatal),
}

impl From<StorageError> for AppendEntriesError {
    fn from(s: StorageError) -> Self {
        let f: Fatal = s.into();
        f.into()
    }
}
impl From<StorageError> for VoteError {
    fn from(s: StorageError) -> Self {
        let f: Fatal = s.into();
        f.into()
    }
}
impl From<StorageError> for InstallSnapshotError {
    fn from(s: StorageError) -> Self {
        let f: Fatal = s.into();
        f.into()
    }
}
impl From<StorageError> for ClientReadError {
    fn from(s: StorageError) -> Self {
        let f: Fatal = s.into();
        f.into()
    }
}
impl From<StorageError> for InitializeError {
    fn from(s: StorageError) -> Self {
        let f: Fatal = s.into();
        f.into()
    }
}
impl From<StorageError> for AddLearnerError {
    fn from(s: StorageError) -> Self {
        let f: Fatal = s.into();
        f.into()
    }
}

/// Error variants related to the Replication.
#[derive(Debug, thiserror::Error)]
#[allow(clippy::large_enum_variant)]
pub enum ReplicationError {
    #[error(transparent)]
    HigherTerm(#[from] HigherTerm),

    #[error("Replication is closed")]
    Closed,

    #[error(transparent)]
    LackEntry(#[from] LackEntry),

    #[error(transparent)]
    CommittedAdvanceTooMany(#[from] CommittedAdvanceTooMany),

    // TODO(xp): two sub type: StorageError / TransportError
    // TODO(xp): a sub error for just send_append_entries()
    #[error(transparent)]
    StorageError(#[from] StorageError),

    #[error(transparent)]
    Timeout(#[from] Timeout),

    #[error(transparent)]
    Network(#[from] NetworkError),

    #[error(transparent)]
    RemoteError(#[from] RemoteError<AppendEntriesError>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum RPCError<T: Error> {
    #[error(transparent)]
    Timeout(#[from] Timeout),

    #[error(transparent)]
    Network(#[from] NetworkError),

    #[error(transparent)]
    RemoteError(#[from] RemoteError<T>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("error occur on remote peer {target}: {source}")]
pub struct RemoteError<T: std::error::Error> {
    pub target: NodeId,
    pub source: T,
}

impl<T: std::error::Error> RemoteError<T> {
    pub fn new(target: NodeId, e: T) -> Self {
        Self { target, source: e }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("seen a higher term: {higher} GT mine: {mine}")]
pub struct HigherTerm {
    pub higher: u64,
    pub mine: u64,
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
pub struct Timeout {
    pub action: RPCTypes,
    pub id: NodeId,
    pub target: NodeId,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("store has no log at: {index:?}, last purged: {last_purged_log_id:?}")]
pub struct LackEntry {
    pub index: Option<u64>,
    pub last_purged_log_id: Option<LogId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("has to forward request to: {leader_id:?}")]
pub struct ForwardToLeader {
    pub leader_id: Option<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("snapshot segment id mismatch, expect: {expect}, got: {got}")]
pub struct SnapshotMismatch {
    pub expect: SnapshotSegmentId,
    pub got: SnapshotSegmentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("not enough for a quorum, cluster: {cluster}, got: {got:?}")]
pub struct QuorumNotEnough {
    pub cluster: String,
    pub got: BTreeSet<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("the cluster is already undergoing a configuration change at log {membership_log_id}")]
pub struct InProgress {
    pub membership_log_id: LogId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("to add a member {node_id} first need to add it as learner")]
pub struct LearnerNotFound {
    pub node_id: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("replication to learner {node_id} is lagging {distance}, matched: {matched:?}, can not add as member")]
pub struct LearnerIsLagging {
    pub node_id: NodeId,
    pub matched: Option<LogId>,
    pub distance: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("new membership can not be empty")]
pub struct EmptyMembership {}
