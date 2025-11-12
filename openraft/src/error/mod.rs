//! Error types exposed by this crate.

mod allow_next_revert_error;
pub mod decompose;
pub(crate) mod higher_vote;
pub mod into_ok;
pub(crate) mod into_raft_result;
mod invalid_sm;
mod membership_error;
mod node_not_found;
mod operation;
mod replication_closed;
pub(crate) mod replication_error;
pub(crate) mod storage_error;
mod storage_io_result;
mod streaming_error;

mod leader_changed;

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Debug;
use std::time::Duration;

use anyerror::AnyError;
use openraft_macros::since;

pub use self::allow_next_revert_error::AllowNextRevertError;
pub(crate) use self::higher_vote::HigherVote;
pub use self::invalid_sm::InvalidStateMachineType;
pub use self::leader_changed::LeaderChanged;
pub use self::membership_error::MembershipError;
pub use self::node_not_found::NodeNotFound;
pub use self::operation::Operation;
pub use self::replication_closed::ReplicationClosed;
pub(crate) use self::replication_error::ReplicationError;
pub(crate) use self::storage_io_result::StorageIOResult;
pub use self::streaming_error::StreamingError;
use crate::Membership;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::network::RPCTypes;
use crate::raft::AppendEntriesResponse;
use crate::raft_types::SnapshotSegmentId;
use crate::try_as_ref::TryAsRef;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// Error returned by Raft API methods.
///
/// `RaftError` wraps either a [`Fatal`] error indicating the Raft node has stopped (due to storage
/// failure, panic, or shutdown), or an API-specific error `E` (such as [`ClientWriteError`] or
/// [`CheckIsLeaderError`]).
///
/// # Usage
///
/// Match on the error variant to handle appropriately:
///
/// ```ignore
/// match raft.client_write(req).await {
///     Ok(resp) => { /* handle response */ },
///     Err(RaftError::APIError(e)) => {
///         // Handle API error (e.g., forward to leader)
///     }
///     Err(RaftError::Fatal(f)) => {
///         // Raft stopped - initiate shutdown
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum RaftError<C, E = Infallible>
where C: RaftTypeConfig
{
    /// API-specific error returned by Raft API methods.
    #[error(transparent)]
    APIError(E),

    /// Fatal error indicating the Raft node has stopped.
    // Reset serde trait bound for C but not for E
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    #[error(transparent)]
    Fatal(#[from] Fatal<C>),
}

impl<C> RaftError<C, Infallible>
where C: RaftTypeConfig
{
    /// Convert to a [`Fatal`] error if its `APIError` variant is [`Infallible`],
    /// otherwise panic.
    #[since(version = "0.10.0")]
    pub fn unwrap_fatal(self) -> Fatal<C> {
        self.into_fatal().unwrap()
    }
}

impl<C, E> RaftError<C, E>
where
    C: RaftTypeConfig,
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
    pub fn fatal(&self) -> Option<&Fatal<C>> {
        match self {
            RaftError::APIError(_) => None,
            RaftError::Fatal(f) => Some(f),
        }
    }

    /// Try to convert self to Fatal error.
    pub fn into_fatal(self) -> Option<Fatal<C>> {
        match self {
            RaftError::APIError(_) => None,
            RaftError::Fatal(f) => Some(f),
        }
    }

    /// Return a reference to ForwardToLeader if Self::APIError contains it.
    pub fn forward_to_leader(&self) -> Option<&ForwardToLeader<C>>
    where E: TryAsRef<ForwardToLeader<C>> {
        match self {
            RaftError::APIError(api_err) => api_err.try_as_ref(),
            RaftError::Fatal(_) => None,
        }
    }

    /// Try to convert self to ForwardToLeader error if APIError is a ForwardToLeader error.
    pub fn into_forward_to_leader(self) -> Option<ForwardToLeader<C>>
    where E: TryInto<ForwardToLeader<C>> {
        match self {
            RaftError::APIError(api_err) => api_err.try_into().ok(),
            RaftError::Fatal(_) => None,
        }
    }
}

impl<C, E> TryAsRef<ForwardToLeader<C>> for RaftError<C, E>
where
    C: RaftTypeConfig,
    E: Debug + TryAsRef<ForwardToLeader<C>>,
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<C>> {
        self.forward_to_leader()
    }
}

impl<C, E> From<StorageError<C>> for RaftError<C, E>
where C: RaftTypeConfig
{
    fn from(se: StorageError<C>) -> Self {
        RaftError::Fatal(Fatal::from(se))
    }
}

/// Unrecoverable error that causes Raft to shut down.
///
/// When a `Fatal` error occurs, the Raft node stops processing requests and enters a stopped state.
/// Applications should monitor for fatal errors and initiate graceful shutdown when detected.
///
/// # Variants
///
/// - `StorageError`: Underlying storage (log or state machine) encountered an error
/// - `Panicked`: Raft core task panicked due to a programming error
/// - `Stopped`: Raft was explicitly shut down via [`Raft::shutdown`]
///
/// [`Raft::shutdown`]: crate::Raft::shutdown
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum Fatal<C>
where C: RaftTypeConfig
{
    /// Storage error that caused the Raft node to stop.
    #[error(transparent)]
    StorageError(#[from] StorageError<C>),

    /// Raft node panicked and stopped.
    #[error("panicked")]
    Panicked,

    /// Raft stopped normally.
    #[error("raft stopped")]
    Stopped,
}

/// Error related to installing a snapshot.
// TODO: remove
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum InstallSnapshotError {
    /// The snapshot segment offset does not match what was expected.
    #[error(transparent)]
    SnapshotMismatch(#[from] SnapshotMismatch),
}

/// An error related to an is_leader request.
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum CheckIsLeaderError<C>
where C: RaftTypeConfig
{
    /// This node is not the leader; request should be forwarded to the leader.
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<C>),

    /// Cannot finish a request, such as elect or replicate, because a quorum is not available.
    #[error(transparent)]
    QuorumNotEnough(#[from] QuorumNotEnough<C>),
}

impl<C> TryAsRef<ForwardToLeader<C>> for CheckIsLeaderError<C>
where C: RaftTypeConfig
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<C>> {
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
pub enum ClientWriteError<C>
where C: RaftTypeConfig
{
    /// This node is not the leader; request should be forwarded to the leader.
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<C>),

    /// When writing a change-membership entry.
    #[error(transparent)]
    ChangeMembershipError(#[from] ChangeMembershipError<C>),
}

impl<C> TryAsRef<ForwardToLeader<C>> for ClientWriteError<C>
where C: RaftTypeConfig
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<C>> {
        match self {
            Self::ForwardToLeader(f) => Some(f),
            _ => None,
        }
    }
}

/// The set of errors which may take place when requesting to propose a config change.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ChangeMembershipError<C: RaftTypeConfig> {
    /// A membership change is already in progress.
    #[error(transparent)]
    InProgress(#[from] InProgress<C>),

    /// The proposed membership change would result in an empty membership.
    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    /// A learner that should be in the cluster was not found.
    #[error(transparent)]
    LearnerNotFound(#[from] LearnerNotFound<C>),
}

/// The set of errors which may take place when initializing a pristine Raft node.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, derive_more::TryInto)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum InitializeError<C>
where C: RaftTypeConfig
{
    /// Initialization operation is not allowed in the current state.
    #[error(transparent)]
    NotAllowed(#[from] NotAllowed<C>),

    /// This node is not included in the initial membership configuration.
    #[error(transparent)]
    NotInMembers(#[from] NotInMembers<C>),
}

/// Error occurs when invoking a remote raft API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
// C already has serde bound.
// E still needs additional serde bound.
// `serde(bound="")` does not work in this case.
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(bound(serialize = "E: serde::Serialize")),
    serde(bound(deserialize = "E: for <'d> serde::Deserialize<'d>"))
)]
pub enum RPCError<C: RaftTypeConfig, E: Error = Infallible> {
    /// The RPC request timed out.
    #[error(transparent)]
    Timeout(#[from] Timeout<C>),

    /// The node is temporarily unreachable and should backoff before retrying.
    #[error(transparent)]
    Unreachable(#[from] Unreachable),

    /// Failed to send the RPC request and should retry immediately.
    #[error(transparent)]
    Network(#[from] NetworkError),

    /// The remote node returned an error.
    #[error(transparent)]
    RemoteError(#[from] RemoteError<C, E>),
}

impl<C, E> RPCError<C, RaftError<C, E>>
where
    C: RaftTypeConfig,
    E: Error,
{
    /// Return a reference to ForwardToLeader error if Self::RemoteError contains one.
    pub fn forward_to_leader(&self) -> Option<&ForwardToLeader<C>>
    where E: TryAsRef<ForwardToLeader<C>> {
        match self {
            RPCError::Timeout(_) => None,
            RPCError::Unreachable(_) => None,
            RPCError::Network(_) => None,
            RPCError::RemoteError(remote_err) => remote_err.source.forward_to_leader(),
        }
    }
}

impl<C> RPCError<C>
where C: RaftTypeConfig
{
    /// Convert to a [`RPCError`] with [`RaftError`] as the error type.
    pub fn with_raft_error<E: Error>(self) -> RPCError<C, RaftError<C, E>> {
        match self {
            RPCError::Timeout(e) => RPCError::Timeout(e),
            RPCError::Unreachable(e) => RPCError::Unreachable(e),
            RPCError::Network(e) => RPCError::Network(e),
        }
    }
}

/// Error that occurred on a remote Raft peer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("error occur on remote peer {target}: {source}")]
pub struct RemoteError<C, T: Error>
where C: RaftTypeConfig
{
    /// The node ID of the remote peer where the error occurred.
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    pub target: C::NodeId,
    /// The node information of the remote peer, if available.
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    pub target_node: Option<C::Node>,
    /// The error that occurred on the remote peer.
    pub source: T,
}

impl<C: RaftTypeConfig, T: Error> RemoteError<C, T> {
    /// Create a new RemoteError with target node ID.
    pub fn new(target: C::NodeId, e: T) -> Self {
        Self {
            target,
            target_node: None,
            source: e,
        }
    }
    /// Create a new RemoteError with target node ID and node information.
    pub fn new_with_node(target: C::NodeId, node: C::Node, e: T) -> Self {
        Self {
            target,
            target_node: Some(node),
            source: e,
        }
    }
}

impl<C, E> From<RemoteError<C, Fatal<C>>> for RemoteError<C, RaftError<C, E>>
where
    C: RaftTypeConfig,
    E: Error,
{
    fn from(e: RemoteError<C, Fatal<C>>) -> Self {
        RemoteError {
            target: e.target,
            target_node: e.target_node,
            source: RaftError::Fatal(e.source),
        }
    }
}

/// Error that indicates a **temporary** network error and when it is returned, Openraft will retry
/// immediately.
///
/// Unlike [`Unreachable`], which indicates an error that should backoff before retrying.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("NetworkError: {source}")]
pub struct NetworkError {
    #[from]
    source: AnyError,
}

impl NetworkError {
    /// Create a new NetworkError from an error.
    pub fn new<E: Error + 'static>(e: &E) -> Self {
        Self {
            source: AnyError::new(e),
        }
    }
}

/// Error indicating a node is unreachable. Retries should be delayed.
///
/// This error suggests that immediate retries are not advisable when a node is not reachable.
/// Upon encountering this error, Openraft will invoke [`backoff()`] to implement a delay before
/// attempting to resend any information.
///
/// This error is similar to [`NetworkError`] but with a key distinction: `Unreachable` advises a
/// backoff period, whereas with [`NetworkError`], Openraft may attempt an immediate retry.
///
/// [`backoff()`]: crate::network::RaftNetwork::backoff
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("Unreachable node: {source}")]
pub struct Unreachable {
    #[from]
    source: AnyError,
}

impl Unreachable {
    /// Create a new Unreachable error from an error.
    pub fn new<E: Error + 'static>(e: &E) -> Self {
        Self {
            source: AnyError::new(e),
        }
    }
}

/// Error indicating that an RPC request timed out.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("timeout after {timeout:?} when {action} {id}->{target}")]
pub struct Timeout<C: RaftTypeConfig> {
    /// The type of RPC that timed out.
    pub action: RPCTypes,
    /// The node ID that initiated the request.
    pub id: C::NodeId,
    /// The target node ID.
    pub target: C::NodeId,
    /// The timeout duration that elapsed.
    pub timeout: Duration,
}

/// Error indicating that the request should be forwarded to the leader.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("has to forward request to: {leader_id:?}, {leader_node:?}")]
pub struct ForwardToLeader<C>
where C: RaftTypeConfig
{
    /// The node ID of the current leader, if known.
    pub leader_id: Option<C::NodeId>,
    /// The node information of the current leader, if known.
    pub leader_node: Option<C::Node>,
}

impl<C> ForwardToLeader<C>
where C: RaftTypeConfig
{
    /// Create a ForwardToLeader error with no known leader information.
    pub const fn empty() -> Self {
        Self {
            leader_id: None,
            leader_node: None,
        }
    }

    /// Create a ForwardToLeader error with known leader information.
    pub fn new(leader_id: C::NodeId, node: C::Node) -> Self {
        Self {
            leader_id: Some(leader_id),
            leader_node: Some(node),
        }
    }
}

/// Error indicating a snapshot segment ID mismatch.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("snapshot segment id mismatch, expect: {expect}, got: {got}")]
pub struct SnapshotMismatch {
    /// The expected snapshot segment ID.
    pub expect: SnapshotSegmentId,
    /// The actual snapshot segment ID received.
    pub got: SnapshotSegmentId,
}

/// Error indicating that not enough nodes responded to form a quorum.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("not enough for a quorum, cluster: {cluster}, got: {got:?}")]
pub struct QuorumNotEnough<C: RaftTypeConfig> {
    /// A description of the cluster membership.
    pub cluster: String,
    /// The set of nodes that responded.
    pub got: BTreeSet<C::NodeId>,
}

/// Error indicating a membership change is already in progress.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error(
    "the cluster is already undergoing a configuration change at log {membership_log_id:?}, last committed membership log id: {committed:?}"
)]
pub struct InProgress<C: RaftTypeConfig> {
    /// The log ID of the last committed membership change.
    pub committed: Option<LogIdOf<C>>,
    /// The log ID of the membership change currently in progress.
    pub membership_log_id: Option<LogIdOf<C>>,
}

/// Error indicating a learner node was not found in the cluster.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("Learner {node_id} not found: add it as learner before adding it as a voter")]
pub struct LearnerNotFound<C: RaftTypeConfig> {
    /// The node ID of the learner that was not found.
    pub node_id: C::NodeId,
}

/// Error indicating an operation is not allowed in the current state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("not allowed to initialize due to current raft state: last_log_id: {last_log_id:?} vote: {vote}")]
pub struct NotAllowed<C: RaftTypeConfig> {
    /// The last log ID in the current state.
    pub last_log_id: Option<LogIdOf<C>>,
    /// The current vote state.
    pub vote: VoteOf<C>,
}

/// Error indicating a node is not a member of the cluster.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("node {node_id} has to be a member. membership:{membership:?}")]
pub struct NotInMembers<C>
where C: RaftTypeConfig
{
    /// The node ID that is not in the membership.
    pub node_id: C::NodeId,
    /// The current cluster membership.
    pub membership: Membership<C>,
}

/// Error indicating an empty membership configuration was provided.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("new membership cannot be empty")]
pub struct EmptyMembership {}

/// An error type that can never occur, used as a placeholder for infallible operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("infallible")]
pub enum Infallible {}

/// A placeholder to mark RaftError won't have a ForwardToLeader variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("no-forward")]
pub enum NoForward {}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub(crate) enum RejectVoteRequest<C: RaftTypeConfig> {
    #[error("reject vote request by a greater vote: {0}")]
    ByVote(VoteOf<C>),

    #[allow(dead_code)]
    #[error("reject vote request by a greater last-log-id: {0:?}")]
    ByLastLogId(Option<LogIdOf<C>>),
}

impl<C> From<RejectVoteRequest<C>> for AppendEntriesResponse<C>
where C: RaftTypeConfig
{
    fn from(r: RejectVoteRequest<C>) -> Self {
        match r {
            RejectVoteRequest::ByVote(v) => AppendEntriesResponse::HigherVote(v),
            RejectVoteRequest::ByLastLogId(_) => {
                unreachable!("the leader should always has a greater last log id")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RejectAppendEntries<C: RaftTypeConfig> {
    #[error("reject AppendEntries by a greater vote: {0}")]
    ByVote(VoteOf<C>),

    #[error("reject AppendEntries because of conflicting log-id: {local:?}; expect to be: {expect:?}")]
    ByConflictingLogId {
        expect: LogIdOf<C>,
        local: Option<LogIdOf<C>>,
    },
}

impl<C> From<RejectVoteRequest<C>> for RejectAppendEntries<C>
where C: RaftTypeConfig
{
    fn from(r: RejectVoteRequest<C>) -> Self {
        match r {
            RejectVoteRequest::ByVote(v) => RejectAppendEntries::ByVote(v),
            RejectVoteRequest::ByLastLogId(_) => {
                unreachable!("the leader should always has a greater last log id")
            }
        }
    }
}

impl<C> From<Result<(), RejectAppendEntries<C>>> for AppendEntriesResponse<C>
where C: RaftTypeConfig
{
    fn from(r: Result<(), RejectAppendEntries<C>>) -> Self {
        match r {
            Ok(_) => AppendEntriesResponse::Success,
            Err(e) => match e {
                RejectAppendEntries::ByVote(v) => AppendEntriesResponse::HigherVote(v),
                RejectAppendEntries::ByConflictingLogId { expect: _, local: _ } => AppendEntriesResponse::Conflict,
            },
        }
    }
}
