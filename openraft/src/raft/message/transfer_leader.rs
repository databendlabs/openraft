use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// A request to transfer leadership from the current leader to another node.
#[since(version = "0.10.0", change = "added last_log_id")]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    /// The vote of the Leader that is transferring the leadership.
    pub(crate) from_leader: VoteOf<C>,

    /// The assigned node to be the next Leader.
    pub(crate) to_node_id: C::NodeId,

    /// The last log id the `to_node_id` node should at least have to become Leader.
    pub(crate) last_log_id: Option<LogIdOf<C>>,
}

impl<C> TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    /// Create a new transfer leader request.
    #[since(version = "0.10.0", change = "added last_log_id arg")]
    pub fn new(from: VoteOf<C>, to: C::NodeId, last_log_id: Option<LogIdOf<C>>) -> Self {
        Self {
            from_leader: from,
            to_node_id: to,
            last_log_id,
        }
    }

    /// From which Leader the leadership is transferred.
    pub fn from_leader(&self) -> &VoteOf<C> {
        &self.from_leader
    }

    /// To which node the leadership is transferred.
    pub fn to_node_id(&self) -> &C::NodeId {
        &self.to_node_id
    }

    /// The last log id on the `to_node_id` node should at least have to become Leader.
    ///
    /// This is the last log id on the Leader when the leadership is transferred.
    #[since(version = "0.10.0")]
    pub fn last_log_id(&self) -> Option<&LogIdOf<C>> {
        self.last_log_id.as_ref()
    }
}

/// The response to [`Raft::handle_transfer_leader()`], which the transferring leader receives as
/// the result of [`NetTransferLeader::transfer_leader()`].
///
/// `Ok(())` means only that the receiver queued the request for its RaftCore:
///
/// - A receiver other than the request's [`to_node_id()`] queues the request without checking it,
///   so it returns `Ok(())` also for a stale request.
/// - The `to_node_id` node, the designated successor, queues the request only after its flushed log
///   progress shows the request's vote and reaches the request's [`last_log_id()`]. Otherwise it
///   returns a [`TransferLeaderError`] and does not queue the request.
///
/// The response does not wait for RaftCore to handle the request. RaftCore ignores the request if
/// the receiver's vote differs from the request's [`from_leader()`] when RaftCore handles it. If
/// the votes are equal, RaftCore resets the receiver's Leader lease, and on the successor it also
/// starts an election if the successor's last log id is at least the request's [`last_log_id()`].
/// So `Ok(())` does not tell whether an election started or whether leadership moved. To learn
/// that, wait on the metrics, as [handing over leadership] shows.
///
/// [`Raft::handle_transfer_leader()`]: crate::Raft::handle_transfer_leader
/// [`NetTransferLeader::transfer_leader()`]: crate::network::NetTransferLeader::transfer_leader
/// [`to_node_id()`]: TransferLeaderRequest::to_node_id
/// [`last_log_id()`]: TransferLeaderRequest::last_log_id
/// [`from_leader()`]: TransferLeaderRequest::from_leader
/// [handing over leadership]: crate::docs::cluster_control::leadership#handing-over-leadership
#[since(version = "0.10.0")]
pub type TransferLeaderResponse<C> = Result<(), TransferLeaderError<C>>;

/// Non-fatal reason the designated successor, the request's [`to_node_id()`], rejected a
/// transfer-leader request.
///
/// Only the successor checks the request, so other receivers never return this error. The
/// successor compares the request with its log progress, as [`Raft::watch_log_progress()`]
/// publishes it: the vote and the log id of the latest I/O that storage reported as flushed. This
/// progress can lag behind the vote and the log that the successor has already accepted. The
/// request passes when the flushed vote equals the request's [`from_leader()`] and the flushed log
/// id is at least the request's [`last_log_id()`]. A rejected request does not reach RaftCore, so
/// the successor neither resets its Leader lease nor starts an election for it.
///
/// [`to_node_id()`]: TransferLeaderRequest::to_node_id
/// [`from_leader()`]: TransferLeaderRequest::from_leader
/// [`last_log_id()`]: TransferLeaderRequest::last_log_id
/// [`Raft::watch_log_progress()`]: crate::Raft::watch_log_progress
#[since(version = "0.10.0")]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[derive(thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum TransferLeaderError<C>
where C: RaftTypeConfig
{
    /// The successor flushed a vote that is greater than the request's vote, or incomparable with
    /// it, such as the vote of a newer candidate or Leader.
    ///
    /// The flushed vote never decreases, so the request can no longer pass, and the successor
    /// returns this error as soon as it observes such a vote.
    #[error("transfer leader rejected because vote changed; expected: {expected:?}, actual: {actual:?}")]
    VoteChanged {
        /// The leader vote carried by the transfer request.
        expected: VoteOf<C>,
        /// The [`FlushPoint::vote`] in the successor's log progress when it rejected the request.
        ///
        /// [`FlushPoint::vote`]: crate::raft::FlushPoint::vote
        actual: VoteOf<C>,
    },

    /// The request did not pass the successor's check within [`Config::election_timeout_min`]
    /// after it arrived.
    ///
    /// When the time ran out, the successor had published no log progress yet, its flushed vote
    /// was still lower than the request's vote, or its flushed vote equaled the request's vote but
    /// `actual` was below `expected`. With a lower flushed vote, `actual` can already reach
    /// `expected`.
    ///
    /// [`Config::election_timeout_min`]: crate::Config::election_timeout_min
    #[error("transfer leader rejected because log is not flushed; expected: {expected:?}, actual: {actual:?}")]
    LogNotFlushed {
        /// The last log id from the transferring leader.
        expected: Option<LogIdOf<C>>,
        /// The [`FlushPoint::last_log_id`] in the successor's log progress when it rejected the
        /// request. It can differ from the last log id in the successor's storage.
        ///
        /// It is `None` if the successor has published no log progress yet, or if its latest flush
        /// point holds no log id, such as the one published at startup or one for an I/O that
        /// saved only a vote. The successor can still hold older entries then, so `None` does not
        /// mean that its log is empty.
        ///
        /// [`FlushPoint::last_log_id`]: crate::raft::FlushPoint::last_log_id
        actual: Option<LogIdOf<C>>,
    },
}

impl<C> fmt::Display for TransferLeaderRequest<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(from_leader={}, to={}, last_log_id={})",
            self.from_leader,
            self.to_node_id,
            self.last_log_id.display()
        )
    }
}
