//! Error types exposed by this crate.

mod allow_next_revert_error;
pub mod decompose;
pub mod into_ok;
pub(crate) mod into_raft_result;
mod invalid_sm;
mod membership_error;
mod node_not_found;
mod operation;
mod replication_closed;
mod streaming_error;

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fmt::Debug;
use std::time::Duration;

use anyerror::AnyError;
use openraft_macros::since;

pub use self::allow_next_revert_error::AllowNextRevertError;
pub use self::invalid_sm::InvalidStateMachineType;
pub use self::membership_error::MembershipError;
pub use self::node_not_found::NodeNotFound;
pub use self::operation::Operation;
pub use self::replication_closed::ReplicationClosed;
pub use self::streaming_error::StreamingError;
use crate::network::RPCTypes;
use crate::raft::AppendEntriesResponse;
use crate::raft_types::SnapshotSegmentId;
use crate::try_as_ref::TryAsRef;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;
use crate::Membership;
use crate::RaftTypeConfig;
use crate::StorageError;

/// RaftError is returned by API methods of `Raft`.
///
/// It is either a Fatal error indicating that `Raft` is no longer running, such as underlying IO
/// error, or an API error `E`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum RaftError<C, E = Infallible>
where C: RaftTypeConfig
{
    #[error(transparent)]
    APIError(E),

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

/// Fatal is unrecoverable and shuts down raft at once.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum Fatal<C>
where C: RaftTypeConfig
{
    #[error(transparent)]
    StorageError(#[from] StorageError<C>),

    #[error("panicked")]
    Panicked,

    /// Raft stopped normally.
    #[error("raft stopped")]
    Stopped,
}

// TODO: remove
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum InstallSnapshotError {
    #[error(transparent)]
    SnapshotMismatch(#[from] SnapshotMismatch),
}

/// An error related to a is_leader request.
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum CheckIsLeaderError<C>
where C: RaftTypeConfig
{
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<C>),

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
    #[error(transparent)]
    InProgress(#[from] InProgress<C>),

    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    #[error(transparent)]
    LearnerNotFound(#[from] LearnerNotFound<C>),
}

/// The set of errors which may take place when initializing a pristine Raft node.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, derive_more::TryInto)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum InitializeError<C>
where C: RaftTypeConfig
{
    #[error(transparent)]
    NotAllowed(#[from] NotAllowed<C>),

    #[error(transparent)]
    NotInMembers(#[from] NotInMembers<C>),
}

/// Error variants related to the Replication.
#[derive(Debug, thiserror::Error)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ReplicationError<C>
where C: RaftTypeConfig
{
    #[error(transparent)]
    HigherVote(#[from] HigherVote<C>),

    #[error(transparent)]
    Closed(#[from] ReplicationClosed),

    #[error(transparent)]
    StorageError(#[from] StorageError<C>),

    #[error(transparent)]
    RPCError(#[from] RPCError<C>),
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
    #[error(transparent)]
    Timeout(#[from] Timeout<C>),

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
            RPCError::PayloadTooLarge(_) => None,
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
            RPCError::PayloadTooLarge(e) => RPCError::PayloadTooLarge(e),
            RPCError::Network(e) => RPCError::Network(e),
            RPCError::RemoteError(_infallible) => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("error occur on remote peer {target}: {source}")]
pub struct RemoteError<C, T: Error>
where C: RaftTypeConfig
{
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    pub target: C::NodeId,
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    pub target_node: Option<C::Node>,
    pub source: T,
}

impl<C: RaftTypeConfig, T: Error> RemoteError<C, T> {
    pub fn new(target: C::NodeId, e: T) -> Self {
        Self {
            target,
            target_node: None,
            source: e,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("seen a higher vote: {higher} GT mine: {sender_vote}")]
pub(crate) struct HigherVote<C: RaftTypeConfig> {
    pub(crate) higher: VoteOf<C>,
    pub(crate) sender_vote: VoteOf<C>,
}

/// Error that indicates a **temporary** network error and when it is returned, Openraft will retry
/// immediately.
///
/// Unlike [`Unreachable`], which indicates a error that should backoff before retrying.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
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
///     ) -> Result<_, RPCError<C::NodeId, C::Node, RaftError<C>>> {
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
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
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
            RPCTypes::TransferLeader => {
                unreachable!("TransferLeader rpc should not have payload")
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
pub struct Timeout<C: RaftTypeConfig> {
    pub action: RPCTypes,
    pub id: C::NodeId,
    pub target: C::NodeId,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("has to forward request to: {leader_id:?}, {leader_node:?}")]
pub struct ForwardToLeader<C>
where C: RaftTypeConfig
{
    pub leader_id: Option<C::NodeId>,
    pub leader_node: Option<C::Node>,
}

impl<C> ForwardToLeader<C>
where C: RaftTypeConfig
{
    pub const fn empty() -> Self {
        Self {
            leader_id: None,
            leader_node: None,
        }
    }

    pub fn new(leader_id: C::NodeId, node: C::Node) -> Self {
        Self {
            leader_id: Some(leader_id),
            leader_node: Some(node),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[error("snapshot segment id mismatch, expect: {expect}, got: {got}")]
pub struct SnapshotMismatch {
    pub expect: SnapshotSegmentId,
    pub got: SnapshotSegmentId,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("not enough for a quorum, cluster: {cluster}, got: {got:?}")]
pub struct QuorumNotEnough<C: RaftTypeConfig> {
    pub cluster: String,
    pub got: BTreeSet<C::NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("the cluster is already undergoing a configuration change at log {membership_log_id:?}, last committed membership log id: {committed:?}")]
pub struct InProgress<C: RaftTypeConfig> {
    pub committed: Option<LogIdOf<C>>,
    pub membership_log_id: Option<LogIdOf<C>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("Learner {node_id} not found: add it as learner before adding it as a voter")]
pub struct LearnerNotFound<C: RaftTypeConfig> {
    pub node_id: C::NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("not allowed to initialize due to current raft state: last_log_id: {last_log_id:?} vote: {vote}")]
pub struct NotAllowed<C: RaftTypeConfig> {
    pub last_log_id: Option<LogIdOf<C>>,
    pub vote: VoteOf<C>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("node {node_id} has to be a member. membership:{membership:?}")]
pub struct NotInMembers<C>
where C: RaftTypeConfig
{
    pub node_id: C::NodeId,
    pub membership: Membership<C>,
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
