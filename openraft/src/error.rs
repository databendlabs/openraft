//! Error types exposed by this crate.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fmt::Debug;
use std::time::Duration;

use anyerror::AnyError;

use crate::network::RPCTypes;
use crate::node::Node;
use crate::raft::AppendEntriesResponse;
use crate::raft_types::SnapshotSegmentId;
use crate::try_as_ref::TryAsRef;
use crate::LogId;
use crate::Membership;
use crate::NodeId;
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

    /// Raft stopped normally.
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

    #[error(transparent)]
    Closed(#[from] ReplicationClosed),

    // TODO(xp): two sub type: StorageError / TransportError
    // TODO(xp): a sub error for just send_append_entries()
    #[error(transparent)]
    StorageError(#[from] StorageError<NID>),

    #[error(transparent)]
    RPCError(#[from] RPCError<NID, N, RaftError<NID, Infallible>>),
}

/// Error occurs when replication is closed.
#[derive(Debug, thiserror::Error)]
#[error("Replication is closed by RaftCore")]
pub(crate) struct ReplicationClosed {}

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

    /// The node is temporarily unreachable and should backoff before retrying.
    #[error(transparent)]
    Unreachable(#[from] Unreachable),

    /// The RPC payload is too large and should be split into smaller chunks.
    #[error(transparent)]
    PayloadTooLarge(#[from] PayloadTooLarge),

    /// Failed to send the RPC request and should retry immediately.
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
            RPCError::Unreachable(_) => None,
            RPCError::PayloadTooLarge(_) => None,
            RPCError::Network(_) => None,
            RPCError::RemoteError(remote_err) => remote_err.source.forward_to_leader(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("error occur on remote peer {target}: {source}")]
pub struct RemoteError<NID: NodeId, N: Node, T: Error> {
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    pub target: NID,
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    pub target_node: Option<N>,
    pub source: T,
}

impl<NID: NodeId, N: Node, T: Error> RemoteError<NID, N, T> {
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

/// Error that indicates a **temporary** network error and when it is returned, Openraft will retry
/// immediately.
///
/// Unlike [`Unreachable`], which indicates a error that should backoff before retrying.
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
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("Unreachable node: {source}")]
pub struct Unreachable {
    #[from]
    source: AnyError,
}

impl Unreachable {
    pub fn new<E: Error + 'static>(e: &E) -> Self {
        Self {
            source: AnyError::new(e),
        }
    }
}

/// Error indicating that an RPC is too large and cannot be sent.
///
/// This is a retryable error:
/// A [`RaftNetwork`] implementation returns this error to inform Openraft to divide an
/// [`AppendEntriesRequest`] into smaller chunks.
/// Openraft will immediately retry sending in smaller chunks.
/// If the request cannot be divided(contains only one entry), Openraft interprets it as
/// [`Unreachable`].
///
/// A hint can be provided to help Openraft in splitting the request.
///
/// The application should also set an appropriate value for [`Config::max_payload_entries`]  to
/// avoid returning this error if possible.
///
/// Example:
///
/// ```ignore
/// impl<C: RaftTypeConfig> RaftNetwork<C> for MyNetwork {
///     fn append_entries(&self,
///             rpc: AppendEntriesRequest<C>,
///             option: RPCOption
///     ) -> Result<_, RPCError<C::NodeId, C::Node, RaftError<C::NodeId>>> {
///         if rpc.entries.len() > 10 {
///             return Err(PayloadTooLarge::new_entries_hint(10).into());
///         }
///         // ...
///     }
/// }
/// ```
///
/// [`RaftNetwork`]: crate::network::RaftNetwork
/// [`AppendEntriesRequest`]: crate::raft::AppendEntriesRequest
/// [`Config::max_payload_entries`]: crate::config::Config::max_payload_entries
///
/// [`InstallSnapshotRequest`]: crate::raft::InstallSnapshotRequest
/// [`Config::snapshot_max_chunk_size`]: crate::config::Config::snapshot_max_chunk_size
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct PayloadTooLarge {
    action: RPCTypes,

    /// An optional hint indicating the anticipated number of entries.
    /// Used only for append-entries replication.
    entries_hint: u64,

    /// An optional hint indicating the anticipated size in bytes.
    /// Used for snapshot replication.
    bytes_hint: u64,

    #[source]
    source: Option<AnyError>,
}

impl fmt::Display for PayloadTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RPC",)?;
        write!(f, "({})", self.action)?;
        write!(f, " payload too large:",)?;

        write!(f, " hint:(")?;
        match self.action {
            RPCTypes::Vote => {
                unreachable!("vote rpc should not have payload")
            }
            RPCTypes::AppendEntries => {
                write!(f, "entries:{}", self.entries_hint)?;
            }
            RPCTypes::InstallSnapshot => {
                write!(f, "bytes:{}", self.bytes_hint)?;
            }
        }
        write!(f, ")")?;

        if let Some(s) = &self.source {
            write!(f, ", source: {}", s)?;
        }

        Ok(())
    }
}

impl PayloadTooLarge {
    /// Create a new PayloadTooLarge, with entries hint, without the causing error.
    pub fn new_entries_hint(entries_hint: u64) -> Self {
        debug_assert!(entries_hint > 0, "entries_hint should be greater than 0");

        Self {
            action: RPCTypes::AppendEntries,
            entries_hint,
            bytes_hint: u64::MAX,
            source: None,
        }
    }

    // No used yet.
    /// Create a new PayloadTooLarge, with bytes hint, without the causing error.
    #[allow(dead_code)]
    pub(crate) fn new_bytes_hint(bytes_hint: u64) -> Self {
        debug_assert!(bytes_hint > 0, "bytes_hint should be greater than 0");

        Self {
            action: RPCTypes::InstallSnapshot,
            entries_hint: u64::MAX,
            bytes_hint,
            source: None,
        }
    }

    /// Set the source error that causes this PayloadTooLarge error.
    pub fn with_source_error(mut self, e: &(impl Error + 'static)) -> Self {
        self.source = Some(AnyError::new(e));
        self
    }

    pub fn action(&self) -> RPCTypes {
        self.action
    }

    /// Get the hint for entries number.
    pub fn entries_hint(&self) -> u64 {
        self.entries_hint
    }

    // No used yet.
    #[allow(dead_code)]
    pub(crate) fn bytes_hint(&self) -> u64 {
        self.bytes_hint
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

impl<NID, N> ForwardToLeader<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub const fn empty() -> Self {
        Self {
            leader_id: None,
            leader_node: None,
        }
    }

    pub fn new(leader_id: NID, node: N) -> Self {
        Self {
            leader_id: Some(leader_id),
            leader_node: Some(node),
        }
    }
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
#[error("the cluster is already undergoing a configuration change at log {membership_log_id:?}, last committed membership log id: {committed:?}")]
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

    #[allow(dead_code)]
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RejectAppendEntries<NID: NodeId> {
    #[error("reject AppendEntries by a greater vote: {0}")]
    ByVote(Vote<NID>),

    #[error("reject AppendEntries because of conflicting log-id: {local:?}; expect to be: {expect:?}")]
    ByConflictingLogId {
        expect: LogId<NID>,
        local: Option<LogId<NID>>,
    },
}

impl<NID: NodeId> From<RejectVoteRequest<NID>> for RejectAppendEntries<NID> {
    fn from(r: RejectVoteRequest<NID>) -> Self {
        match r {
            RejectVoteRequest::ByVote(v) => RejectAppendEntries::ByVote(v),
            RejectVoteRequest::ByLastLogId(_) => {
                unreachable!("the leader should always has a greater last log id")
            }
        }
    }
}

impl<NID: NodeId> From<Result<(), RejectAppendEntries<NID>>> for AppendEntriesResponse<NID> {
    fn from(r: Result<(), RejectAppendEntries<NID>>) -> Self {
        match r {
            Ok(_) => AppendEntriesResponse::Success,
            Err(e) => match e {
                RejectAppendEntries::ByVote(v) => AppendEntriesResponse::HigherVote(v),
                RejectAppendEntries::ByConflictingLogId { expect: _, local: _ } => AppendEntriesResponse::Conflict,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use anyerror::AnyError;

    use crate::error::PayloadTooLarge;

    #[test]
    fn test_append_too_large() -> anyhow::Result<()> {
        let a = PayloadTooLarge::new_entries_hint(5);
        assert_eq!("RPC(AppendEntries) payload too large: hint:(entries:5)", a.to_string());

        let a = PayloadTooLarge::new_bytes_hint(5);
        assert_eq!("RPC(InstallSnapshot) payload too large: hint:(bytes:5)", a.to_string());

        let a = PayloadTooLarge::new_entries_hint(5).with_source_error(&AnyError::error("test"));
        assert_eq!(
            "RPC(AppendEntries) payload too large: hint:(entries:5), source: test",
            a.to_string()
        );

        Ok(())
    }
}
