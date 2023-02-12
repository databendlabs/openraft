//! Error types exposed by this crate.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Debug;
use std::time::Duration;

use anyerror::AnyError;

use crate::node::Node;
use crate::raft::AppendEntriesResponse;
use crate::raft_types::SnapshotSegmentId;
use crate::try_as_ref::TryAsRef;
use crate::LogId;
use crate::Membership;
use crate::NodeId;
use crate::RPCTypes;
use crate::StorageError;
use crate::Vote;

/// RaftError is returned by API methods of `Raft`.
///
/// It is either a Fatal error indicating that `Raft` is no longer running, such as underlying IO
/// error, or an API error `E`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound = "E:serde::Serialize + for <'d> serde::Deserialize<'d>")
)]
pub enum RaftError<NID, E = Infallible>
where NID: NodeId
{
    #[error(transparent)]
    APIError(E),

    #[error(transparent)]
    Fatal(#[from] Fatal<NID>),
}

impl<NID, E> RaftError<NID, E>
where
    NID: NodeId,
    E: Debug,
{
    /// Return a reference to Self::APIError.
    pub fn api_error(&self) -> Option<&E> {
        match self {
            RaftError::APIError(e) => Some(e),
            RaftError::Fatal(_) => None,
        }
    }

    /// Try to convert self to APIError.
    pub fn into_api_error(self) -> Option<E> {
        match self {
            RaftError::APIError(e) => Some(e),
            RaftError::Fatal(_) => None,
        }
    }

    /// Return a reference to Self::Fatal.
    pub fn fatal(&self) -> Option<&Fatal<NID>> {
        match self {
            RaftError::APIError(_) => None,
            RaftError::Fatal(f) => Some(f),
        }
    }

    /// Try to convert self to Fatal error.
    pub fn into_fatal(self) -> Option<Fatal<NID>> {
        match self {
            RaftError::APIError(_) => None,
            RaftError::Fatal(f) => Some(f),
        }
    }

    /// Return a reference to ForwardToLeader if Self::APIError contains it.
    pub fn forward_to_leader<N>(&self) -> Option<&ForwardToLeader<NID, N>>
    where
        N: Node,
        E: TryAsRef<ForwardToLeader<NID, N>>,
    {
        match self {
            RaftError::APIError(api_err) => api_err.try_as_ref(),
            RaftError::Fatal(_) => None,
        }
    }

    /// Try to convert self to ForwardToLeader error if APIError is a ForwardToLeader error.
    pub fn into_forward_to_leader<N>(self) -> Option<ForwardToLeader<NID, N>>
    where
        N: Node,
        E: TryInto<ForwardToLeader<NID, N>>,
    {
        match self {
            RaftError::APIError(api_err) => api_err.try_into().ok(),
            RaftError::Fatal(_) => None,
        }
    }
}

/// Fatal is unrecoverable and shuts down raft at once.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum Fatal<NID>
where NID: NodeId
{
    #[error(transparent)]
    StorageError(#[from] StorageError<NID>),

    #[error("panicked")]
    Panicked,

    #[error("raft stopped")]
    Stopped,
}

// TODO: remove
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum InstallSnapshotError {
    #[error(transparent)]
    SnapshotMismatch(#[from] SnapshotMismatch),
}

/// An error related to a is_leader request.
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum CheckIsLeaderError<NID, N>
where
    NID: NodeId,
    N: Node,
{
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<NID, N>),

    #[error(transparent)]
    QuorumNotEnough(#[from] QuorumNotEnough<NID>),
}

impl<NID, N> TryAsRef<ForwardToLeader<NID, N>> for CheckIsLeaderError<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<NID, N>> {
        match self {
            Self::ForwardToLeader(f) => Some(f),
            _ => None,
        }
    }
}

/// An error related to a client write request.
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ClientWriteError<NID, N>
where
    NID: NodeId,
    N: Node,
{
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<NID, N>),

    /// When writing a change-membership entry.
    #[error(transparent)]
    ChangeMembershipError(#[from] ChangeMembershipError<NID>),
}

impl<NID, N> TryAsRef<ForwardToLeader<NID, N>> for ClientWriteError<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<NID, N>> {
        match self {
            Self::ForwardToLeader(f) => Some(f),
            _ => None,
        }
    }
}

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ChangeMembershipError<NID: NodeId> {
    #[error(transparent)]
    InProgress(#[from] InProgress<NID>),

    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    #[error(transparent)]
    LearnerNotFound(#[from] LearnerNotFound<NID>),

    #[error(transparent)]
    LearnerIsLagging(#[from] LearnerIsLagging<NID>),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum AddLearnerError<NID, N>
where
    NID: NodeId,
    N: Node,
{
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<NID, N>),
}

impl<NID, N> TryAsRef<ForwardToLeader<NID, N>> for AddLearnerError<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<NID, N>> {
        match self {
            Self::ForwardToLeader(f) => Some(f),
        }
    }
}

impl<NID, N> TryFrom<AddLearnerError<NID, N>> for ForwardToLeader<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = AddLearnerError<NID, N>;

    fn try_from(value: AddLearnerError<NID, N>) -> Result<Self, Self::Error> {
        let AddLearnerError::ForwardToLeader(e) = value;
        Ok(e)
    }
}

/// The set of errors which may take place when initializing a pristine Raft node.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, derive_more::TryInto)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum InitializeError<NID, N>
where
    NID: NodeId,
    N: Node,
{
    #[error(transparent)]
    NotAllowed(#[from] NotAllowed<NID>),

    #[error(transparent)]
    NotInMembers(#[from] NotInMembers<NID, N>),
}

/// Error variants related to the Replication.
#[derive(Debug, thiserror::Error)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ReplicationError<NID, N>
where
    NID: NodeId,
    N: Node,
{
    #[error(transparent)]
    HigherVote(#[from] HigherVote<NID>),

    #[error("Replication is closed")]
    Closed,

    // TODO(xp): two sub type: StorageError / TransportError
    // TODO(xp): a sub error for just send_append_entries()
    #[error(transparent)]
    StorageError(#[from] StorageError<NID>),

    #[error(transparent)]
    RPCError(#[from] RPCError<NID, N, RaftError<NID, Infallible>>),
}

/// Error occurs when invoking a remote raft API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound = "E:serde::Serialize + for <'d> serde::Deserialize<'d>")
)]
pub enum RPCError<NID: NodeId, N: Node, E: Error> {
    #[error(transparent)]
    Timeout(#[from] Timeout<NID>),

    #[error(transparent)]
    Network(#[from] NetworkError),

    #[error(transparent)]
    RemoteError(#[from] RemoteError<NID, N, E>),
}

impl<NID, N, E> RPCError<NID, N, RaftError<NID, E>>
where
    NID: NodeId,
    N: Node,
    E: Error,
{
    /// Return a reference to ForwardToLeader error if Self::RemoteError contains one.
    pub fn forward_to_leader(&self) -> Option<&ForwardToLeader<NID, N>>
    where E: TryAsRef<ForwardToLeader<NID, N>> {
        match self {
            RPCError::Timeout(_) => None,
            RPCError::Network(_) => None,
            RPCError::RemoteError(remote_err) => remote_err.source.forward_to_leader(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("error occur on remote peer {target}: {source}")]
pub struct RemoteError<NID: NodeId, N: Node, T: std::error::Error> {
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    pub target: NID,
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    pub target_node: Option<N>,
    pub source: T,
}

impl<NID: NodeId, N: Node, T: std::error::Error> RemoteError<NID, N, T> {
    pub fn new(target: NID, e: T) -> Self {
        Self {
            target,
            target_node: None,
            source: e,
        }
    }
    pub fn new_with_node(target: NID, node: N, e: T) -> Self {
        Self {
            target,
            target_node: Some(node),
            source: e,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("seen a higher vote: {higher} GT mine: {mine}")]
pub struct HigherVote<NID: NodeId> {
    pub higher: Vote<NID>,
    pub mine: Vote<NID>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("leader committed index {committed_index} advances target log index {target_index} too many")]
pub struct CommittedAdvanceTooMany {
    pub committed_index: u64,
    pub target_index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("NetworkError: {source}")]
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("timeout after {timeout:?} when {action} {id}->{target}")]
pub struct Timeout<NID: NodeId> {
    pub action: RPCTypes,
    pub id: NID,
    pub target: NID,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("store has no log at: {index:?}, last purged: {last_purged_log_id:?}")]
pub struct LackEntry<NID: NodeId> {
    pub index: Option<u64>,
    pub last_purged_log_id: Option<LogId<NID>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("has to forward request to: {leader_id:?}, {leader_node:?}")]
pub struct ForwardToLeader<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub leader_id: Option<NID>,
    pub leader_node: Option<N>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("snapshot segment id mismatch, expect: {expect}, got: {got}")]
pub struct SnapshotMismatch {
    pub expect: SnapshotSegmentId,
    pub got: SnapshotSegmentId,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("not enough for a quorum, cluster: {cluster}, got: {got:?}")]
pub struct QuorumNotEnough<NID: NodeId> {
    pub cluster: String,
    pub got: BTreeSet<NID>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("the cluster is already undergoing a configuration change at log {membership_log_id:?}, committed log id: {committed:?}")]
pub struct InProgress<NID: NodeId> {
    pub committed: Option<LogId<NID>>,
    pub membership_log_id: Option<LogId<NID>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("Learner {node_id} not found: add it as learner before adding it as a voter")]
pub struct LearnerNotFound<NID: NodeId> {
    pub node_id: NID,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("replication to learner {node_id} is lagging {distance}, matched: {matched:?}, can not add as member")]
pub struct LearnerIsLagging<NID: NodeId> {
    pub node_id: NID,
    pub matched: Option<LogId<NID>>,
    pub distance: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("not allowed to initialize due to current raft state: last_log_id: {last_log_id:?} vote: {vote}")]
pub struct NotAllowed<NID: NodeId> {
    pub last_log_id: Option<LogId<NID>>,
    pub vote: Vote<NID>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("node {node_id} has to be a member. membership:{membership:?}")]
pub struct NotInMembers<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub node_id: NID,
    pub membership: Membership<NID, N>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("new membership can not be empty")]
pub struct EmptyMembership {}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("infallible")]
pub enum Infallible {}

/// A place holder to mark RaftError won't have a ForwardToLeader variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("no-forward")]
pub enum NoForward {}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub(crate) enum RejectVoteRequest<NID: NodeId> {
    #[error("reject vote request by a greater vote: {0}")]
    ByVote(Vote<NID>),

    #[error("reject vote request by a greater last-log-id: {0:?}")]
    ByLastLogId(Option<LogId<NID>>),
}

impl<NID: NodeId> From<RejectVoteRequest<NID>> for AppendEntriesResponse<NID> {
    fn from(r: RejectVoteRequest<NID>) -> Self {
        match r {
            RejectVoteRequest::ByVote(v) => AppendEntriesResponse::HigherVote(v),
            RejectVoteRequest::ByLastLogId(_) => {
                unreachable!("the leader should always has a greater last log id")
            }
        }
    }
}
