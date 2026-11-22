//! Error types exposed by this crate.

mod allow_next_revert_error;
mod conflicting_log_id;
pub mod decompose;
mod error_source;
mod fatal;
pub(crate) mod higher_vote;
pub(crate) mod into_ok;
pub(crate) mod into_raft_result;
mod leader_changed;
mod linearizable_read_error;
mod node_metadata_changed;
mod node_not_found;
mod operation;
mod precondition_failed;
mod raft_error;
mod reject_append_entries;
mod reject_vote;
mod replication_closed;
pub(crate) mod replication_error;
mod shutdown_error;
pub(crate) mod storage_error;
mod storage_io_result;
mod streaming_error;
mod uncommitted_leader_log;
mod unsupported_membership_transition;

use std::collections::BTreeSet;
use std::error::Error;
use std::time::Duration;

use anyerror::AnyError;
use openraft_macros::since;

pub use self::allow_next_revert_error::AllowNextRevertError;
pub use self::conflicting_log_id::ConflictingLogId;
pub use self::error_source::BacktraceDisplay;
pub use self::error_source::ErrorSource;
pub use self::fatal::Fatal;
pub(crate) use self::higher_vote::HigherVote;
pub use self::leader_changed::LeaderChanged;
pub use self::linearizable_read_error::LinearizableReadError;
pub use self::node_metadata_changed::NodeMetadataChanged;
pub use self::node_not_found::NodeNotFound;
pub use self::operation::Operation;
pub use self::precondition_failed::PreconditionFailed;
pub use self::raft_error::RaftError;
pub(crate) use self::reject_append_entries::RejectAppendEntries;
pub use self::reject_vote::RejectVote;
pub use self::replication_closed::ReplicationClosed;
pub(crate) use self::replication_error::ReplicationError;
pub use self::shutdown_error::ShutdownError;
pub(crate) use self::storage_io_result::StorageIOResult;
pub use self::streaming_error::StreamingError;
pub use self::uncommitted_leader_log::UncommittedLeaderLog;
pub use self::unsupported_membership_transition::UnsupportedMembershipTransition;
use crate::LogId;
use crate::RaftTypeConfig;
pub use crate::membership::errors::MembershipError;
pub use crate::membership::errors::UnprovenReadWriteIntersection;
pub use crate::membership::errors::WriteVoterWithoutReceiveLogs;
use crate::network::RPCTypes;
use crate::node::NodeId;
use crate::try_as_ref::TryAsRef;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;
use crate::vote::RaftCommittedLeaderId;

/// For backward compatibility, use [`LinearizableReadError`] instead.
#[deprecated(since = "0.10.0", note = "use `LinearizableReadError` instead")]
pub type CheckIsLeaderError<C> = LinearizableReadError<C>;

/// **REMOVED**: Use `openraft_legacy::network_v1::InstallSnapshotError` instead.
///
/// The v1 chunk-based `InstallSnapshotError` has been moved to the `openraft-legacy` crate.
///
/// # Migration
///
/// Add to `Cargo.toml`:
/// ```toml
/// [dependencies]
/// openraft-legacy = "0.10"
/// ```
///
/// Update imports:
/// ```ignore
/// use openraft_legacy::network_v1::InstallSnapshotError;
/// ```
#[deprecated(
    since = "0.10.0",
    note = "InstallSnapshotError has been moved to the `openraft-legacy` crate. \
            Add `openraft-legacy` to your dependencies and use \
            `openraft_legacy::network_v1::InstallSnapshotError` instead."
)]
pub enum InstallSnapshotError {}

/// An error related to a client write request.
#[since(version = "0.10.0", change = "added `LogEntryDiscarded`")]
#[derive(Debug, Clone, thiserror::Error, derive_more::TryInto)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ClientWriteError<C>
where C: RaftTypeConfig
{
    /// This proposal was rejected before its entry was appended; use this error for routing.
    #[error(transparent)]
    ForwardToLeader(#[from] ForwardToLeader<C>),

    /// The proposal's local log entry was discarded while this node adopted another leader's log.
    ///
    /// The contained [`ForwardToLeader`] provides the latest known leader information. Retrying
    /// requires application-level deduplication because the entry may have committed or may still
    /// commit through another leader.
    #[since(version = "0.10.0")]
    #[error("log entry discarded on leader change; commit status unknown; application must deduplicate retries; {0}")]
    #[try_into(ignore)]
    LogEntryDiscarded(ForwardToLeader<C>),

    /// When writing a change-membership entry.
    #[error(transparent)]
    ChangeMembershipError(#[from] ChangeMembershipError<CommittedLeaderIdOf<C>, C::NodeId, C::ErrorSource>),

    /// A [`Precondition`] attached to the request, or the last log id its [`WriteOption`]
    /// expects, is not satisfied.
    ///
    /// [`Precondition`]: `crate::raft::Precondition`
    /// [`WriteOption`]: `crate::raft::WriteOption`
    #[since(version = "0.10.0")]
    #[error(transparent)]
    PreconditionFailed(#[from] PreconditionFailed<C>),
}

impl<C> TryAsRef<ForwardToLeader<C>> for ClientWriteError<C>
where C: RaftTypeConfig
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<C>> {
        match self {
            Self::ForwardToLeader(f) | Self::LogEntryDiscarded(f) => Some(f),
            _ => None,
        }
    }
}

/// The set of errors which may take place when requesting to propose a config change.
#[since(version = "0.10.0", change = "added `WriteVoterWithoutReceiveLogs` variant")]
#[since(
    version = "0.10.0",
    change = "added `UserDefined` variant and configurable error source `E`"
)]
#[since(version = "0.10.0", change = "added `UnprovenReadWriteIntersection` variant")]
#[since(
    version = "0.10.0",
    change = "added `UnsupportedMembershipTransition`, `UncommittedLeaderLog` and `NodeMetadataChanged` variants"
)]
#[since(
    version = "0.10.0",
    change = "from `ChangeMembershipError<C>` to `ChangeMembershipError<CLID, NID>`"
)]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ChangeMembershipError<CLID, NID, E = AnyError>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    E: ErrorSource,
{
    /// A membership change is already in progress.
    #[error(transparent)]
    InProgress(#[from] InProgress<CLID>),

    /// The proposed membership change would result in an empty membership.
    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    /// A learner that should be in the cluster was not found.
    #[error(transparent)]
    LearnerNotFound(#[from] LearnerNotFound<NID>),

    /// The proposed membership is not a transition a direct membership append supports.
    #[since(version = "0.10.0")]
    #[error(transparent)]
    UnsupportedMembershipTransition(#[from] UnsupportedMembershipTransition),

    /// The leader has not yet committed a log entry proposed in its own term.
    #[since(version = "0.10.0")]
    #[error(transparent)]
    UncommittedLeaderLog(#[from] UncommittedLeaderLog<CLID>),

    /// The proposed membership changes the metadata of a node id the cluster already knows.
    #[since(version = "0.10.0")]
    #[error(transparent)]
    NodeMetadataChanged(#[from] NodeMetadataChanged<NID>),

    /// OpenRaft cannot prove that every read quorum of the proposed membership intersects every
    /// write quorum.
    #[since(version = "0.10.0")]
    #[error(transparent)]
    UnprovenReadWriteIntersection(#[from] UnprovenReadWriteIntersection),

    /// A write voter of the proposed membership does not have [`NodeRole::ReceiveLogs`], so it
    /// never acknowledges a log.
    ///
    /// [`NodeRole::ReceiveLogs`]: crate::membership::NodeRole::ReceiveLogs
    #[since(version = "0.10.0")]
    #[error(transparent)]
    WriteVoterWithoutReceiveLogs(#[from] WriteVoterWithoutReceiveLogs<NID>),

    /// The [`RaftMembership`] implementation rejected the change for a reason of its own.
    ///
    /// [`RaftMembership`]: crate::membership::RaftMembership
    #[since(version = "0.10.2", change = "exposes `E` as the error source")]
    #[since(version = "0.10.0")]
    #[error("user-defined membership error: {0}")]
    UserDefined(#[source] E),
}

/// The set of errors which may take place when initializing a pristine Raft node.
#[since(version = "0.10.0", change = "added `InvalidMembership` variant")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, derive_more::TryInto)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum InitializeError<C>
where C: RaftTypeConfig
{
    /// Initialization operation is not allowed in the current state.
    #[error(transparent)]
    NotAllowed(#[from] NotAllowed<C>),

    /// This node cannot campaign in the initial membership configuration: it is absent from it,
    /// or only a learner in it.
    ///
    /// The initializing node starts an election right after initialization, so it has to be a
    /// [`NodeRole::Campaigner`], such as a voter of a default membership.
    ///
    /// [`NodeRole::Campaigner`]: crate::membership::NodeRole::Campaigner
    #[error(transparent)]
    NotInMembers(#[from] NotInMembers<C>),

    /// The initial membership configuration defines no quorum, a quorum ID has no node, a write
    /// voter does not have [`NodeRole::ReceiveLogs`], or OpenRaft cannot prove that its read
    /// quorums intersect its write quorums.
    ///
    /// [`NodeRole::ReceiveLogs`]: crate::membership::NodeRole::ReceiveLogs
    #[since(version = "0.10.0")]
    #[error(transparent)]
    InvalidMembership(#[from] MembershipError<C::NodeId, C::ErrorSource>),
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

    /// The node is unreachable, and a retry should wait; see [`Unreachable`].
    #[error(transparent)]
    Unreachable(#[from] Unreachable<C>),

    /// A transient transport failure, which a retry may get past; see [`NetworkError`].
    #[error(transparent)]
    Network(#[from] NetworkError<C>),

    /// The remote node returned an error.
    #[error(transparent)]
    RemoteError(#[from] RemoteError<C, E>),
}

impl<C, E> RPCError<C, E>
where
    C: RaftTypeConfig,
    E: Error,
{
    /// Returns the weight this error adds to the error rank of a log replication stream.
    ///
    /// The stream adds up the weights of the failures since its last success, and backs off when
    /// it opens its next session with a rank above a threshold, 20: so one `Unreachable` or
    /// `RemoteError` (100) enables backoff, while `Timeout` and `Network` errors (2) do so only
    /// after more than ten in a row. The weight only decides whether to back off; the
    /// [`Backoff`] iterator gives the delays.
    ///
    /// [`Backoff`]: crate::network::Backoff
    pub(crate) fn backoff_rank(&self) -> u64 {
        match &self {
            RPCError::Timeout(_) => 2,
            RPCError::Unreachable(_unreachable) => 100,
            RPCError::Network(_) => 2,
            RPCError::RemoteError(_) => 100,
        }
    }
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

/// A transient transport failure, such as a lost connection or a failed send, that sending the
/// request again may get past.
///
/// Log replication and snapshot transmission retry such a failure at once, and back off only
/// after more than ten fail in a row. Other RPCs, such as votes and heartbeats, are not retried
/// this way. Return [`Unreachable`] instead for a node that a retry would not reach soon.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("NetworkError: {source}")]
pub struct NetworkError<C: RaftTypeConfig> {
    source: C::ErrorSource,
}

impl<C: RaftTypeConfig> NetworkError<C> {
    /// Create a new NetworkError from an error.
    pub fn new<E: Error + 'static>(e: &E) -> Self {
        Self {
            source: C::ErrorSource::from_error(e),
        }
    }

    /// Create a NetworkError from a string message.
    pub fn from_string(msg: impl ToString) -> Self {
        Self {
            source: C::ErrorSource::from_string(msg),
        }
    }
}

/// Error indicating a node is unreachable, so retries should wait.
///
/// When log replication or snapshot transmission gets this error, it backs off before it sends to
/// the node again, with the delays of the network's [`backoff()`], or of
/// [`Config::backoff`] when the network provides none. A [`NetworkError`] instead is retried at
/// once until several fail in a row.
///
/// [`backoff()`]: crate::network::RaftNetworkV2::backoff
/// [`Config::backoff`]: crate::Config::backoff
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("Unreachable node: {source}")]
pub struct Unreachable<C: RaftTypeConfig> {
    source: C::ErrorSource,
}

impl<C: RaftTypeConfig> Unreachable<C> {
    /// Create a new Unreachable error from an error.
    pub fn new<E: Error + 'static>(e: &E) -> Self {
        Self {
            source: C::ErrorSource::from_error(e),
        }
    }

    /// Create an Unreachable error from a string message.
    pub fn from_string(msg: impl ToString) -> Self {
        Self {
            source: C::ErrorSource::from_string(msg),
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

/// Why a request must be forwarded or retried.
#[since(version = "0.10.0")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum ForwardReason {
    /// The receiving node is not the leader, or not the leader that the request expected.
    ///
    /// A write whose [`WriteOption::with_leader()`] names another committed leader than the
    /// current one gets this reason too, also from the current leader. Its hint names the current
    /// leader, so it can be the receiving node itself, and this reason does not show that the
    /// receiver is a follower. Check whether the expected leader still holds before retrying.
    ///
    /// [`WriteOption::with_leader()`]: crate::raft::WriteOption::with_leader
    #[default]
    NotLeader,

    /// The receiving leader is transferring leadership to another node.
    LeadershipTransfer,

    /// The receiving leader's quorum lease has expired.
    LeaseExpired,
}

/// Routing information for a request this node cannot accept or finish.
///
/// This type does not describe whether a mutating request was accepted. See [`ClientWriteError`]
/// for write outcome semantics.
///
/// [`ClientWriteError`]: crate::errors::ClientWriteError
#[since(version = "0.10.0", change = "added `reason`")]
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
    /// Why the request must be forwarded or retried.
    ///
    /// For [`ForwardReason::LeadershipTransfer`], `leader_id` and `leader_node` identify the
    /// transfer target, which may not have established leadership yet.
    ///
    /// Adding this field is not backward compatible with Serde formats that encode structs
    /// positionally, such as bincode and postcard. Formats that encode structs by field name can
    /// deserialize older data because a missing field defaults to [`ForwardReason::NotLeader`].
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "serde", serde(default))]
    pub reason: ForwardReason,
}

impl<C> ForwardToLeader<C>
where C: RaftTypeConfig
{
    /// Create a ForwardToLeader error with no known leader information.
    pub const fn empty() -> Self {
        Self {
            leader_id: None,
            leader_node: None,
            reason: ForwardReason::NotLeader,
        }
    }

    /// Create a ForwardToLeader error with known leader information.
    pub fn new(leader_id: C::NodeId, node: C::Node) -> Self {
        Self {
            leader_id: Some(leader_id),
            leader_node: Some(node),
            reason: ForwardReason::NotLeader,
        }
    }

    /// Set why the request was rejected.
    #[since(version = "0.10.0")]
    #[must_use]
    pub const fn with_reason(mut self, reason: ForwardReason) -> Self {
        self.reason = reason;
        self
    }
}

/// **REMOVED**: Use `openraft_legacy::network_v1::SnapshotMismatch` instead.
///
/// The v1 chunk-based `SnapshotMismatch` has been moved to the `openraft-legacy` crate.
///
/// # Migration
///
/// Add to `Cargo.toml`:
/// ```toml
/// [dependencies]
/// openraft-legacy = "0.10"
/// ```
///
/// Update imports:
/// ```ignore
/// use openraft_legacy::network_v1::SnapshotMismatch;
/// ```
#[deprecated(
    since = "0.10.0",
    note = "SnapshotMismatch has been moved to the `openraft-legacy` crate. \
            Add `openraft-legacy` to your dependencies and use \
            `openraft_legacy::network_v1::SnapshotMismatch` instead."
)]
pub struct SnapshotMismatch {}

/// Error indicating that not enough nodes responded to form a quorum.
#[since(
    version = "0.10.0",
    change = "`cluster` from a membership description to the write quorum set formatted with `Debug`"
)]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("not enough for a quorum, write quorum set: {cluster}, got: {got:?}")]
pub struct QuorumNotEnough<C: RaftTypeConfig> {
    /// The write quorum set, formatted with `Debug`.
    pub cluster: String,
    /// The set of nodes that responded.
    pub got: BTreeSet<C::NodeId>,
}

/// Error indicating a membership change is already in progress.
#[since(version = "0.10.0", change = "from `InProgress<C>` to `InProgress<CLID>`")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error(
    "the cluster is already undergoing a configuration change at log {membership_log_id:?}, last committed membership log id: {committed:?}"
)]
pub struct InProgress<CLID>
where CLID: RaftCommittedLeaderId
{
    /// The log ID of the last committed membership change.
    pub committed: Option<LogId<CLID>>,
    /// The log ID of the membership change currently in progress.
    pub membership_log_id: Option<LogId<CLID>>,
}

/// Error indicating a learner node was not found in the cluster.
#[since(version = "0.10.0", change = "removed `C: RaftTypeConfig` generic parameter")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("Learner {node_id} not found: add it as learner before adding it as a voter")]
pub struct LearnerNotFound<NID>
where NID: NodeId
{
    /// The node ID of the learner that was not found.
    pub node_id: NID,
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

/// Error indicating that the initializing node cannot campaign in the initial membership: it is
/// absent from it, or only a learner in it.
///
/// See [`InitializeError::NotInMembers`].
#[since(
    version = "0.10.2",
    change = "the message says that the node has to be able to campaign, not only to be a member"
)]
#[since(version = "0.10.0", change = "`membership` field has type `C::Membership`")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("node {node_id} has to be able to campaign in the initial membership, as a voter does: {membership:?}")]
pub struct NotInMembers<C>
where C: RaftTypeConfig
{
    /// The ID of the node that initializes the cluster.
    pub node_id: C::NodeId,
    /// The initial membership configuration passed to `initialize()`.
    pub membership: C::Membership,
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

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fmt;

    use anyerror::AnyError;

    use super::ChangeMembershipError;
    use super::ClientWriteError;
    use super::RaftError;
    use crate::RaftTypeConfig;
    use crate::engine::testing::UTConfig;
    use crate::type_config::alias::CommittedLeaderIdOf;

    type C = UTConfig;

    #[derive(Debug, thiserror::Error)]
    #[error("edit rejected")]
    struct EditRejected(#[source] fmt::Error);

    /// A user-defined membership error is the source of the errors that wrap it, and its own
    /// causes follow it in the chain.
    #[test]
    fn test_user_defined_membership_error_is_the_source() {
        let user = AnyError::new(&EditRejected(fmt::Error));
        let err = ChangeMembershipError::<CommittedLeaderIdOf<C>, u64, <C as RaftTypeConfig>::ErrorSource>::UserDefined(
            user.clone(),
        );
        let write_err = ClientWriteError::<C>::ChangeMembershipError(err.clone());
        let raft_err = RaftError::<C, ClientWriteError<C>>::APIError(write_err.clone());

        let sources: [&dyn Error; 3] = [&err, &write_err, &raft_err];
        for wrapper in sources {
            let source = wrapper.source().and_then(|s| s.downcast_ref::<AnyError>());
            assert_eq!(Some(&user), source, "the source of: {}", wrapper);

            let cause = source.and_then(|s| s.source()).map(|s| s.to_string());
            assert_eq!(Some(fmt::Error.to_string()), cause, "the cause of: {}", wrapper);
        }
    }
}
