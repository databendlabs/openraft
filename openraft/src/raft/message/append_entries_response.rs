use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::raft::StreamAppendError;
use crate::raft::stream_append::StreamAppendResult;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// The response to an `AppendEntriesRequest`.
///
/// [`RaftNetworkV2::append_entries`] returns this type only when received an RPC reply.
/// Otherwise, it should return [`RPCError`].
///
/// [`RPCError`]: crate::error::RPCError
/// [`RaftNetworkV2::append_entries`]: crate::network::RaftNetworkV2::append_entries
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum AppendEntriesResponse<C: RaftTypeConfig> {
    /// Successfully replicated all log entries to the target node.
    Success,

    /// Successfully sent the first portion of log entries.
    ///
    /// [`RaftNetworkV2::append_entries`] can return a partial success.
    /// For example, it tries to send log entries `[1-2..3-10]`, the application is allowed to send
    /// just `[1-2..1-3]` and return `PartialSuccess(1-3)`
    ///
    /// ### Caution
    ///
    /// The returned matching log id must be **greater than or equal to** the first log
    /// id([`AppendEntriesRequest::prev_log_id`]) of the entries to send. If no RPC reply is
    /// received, [`RaftNetworkV2::append_entries`] must return an [`RPCError`] to inform
    /// Openraft that the first log id([`AppendEntriesRequest::prev_log_id`]) may not match on
    /// the remote target node.
    ///
    /// [`RPCError`]: crate::error::RPCError
    /// [`RaftNetworkV2::append_entries`]: crate::network::RaftNetworkV2::append_entries
    /// [`AppendEntriesRequest::prev_log_id`]: crate::raft::AppendEntriesRequest::prev_log_id
    PartialSuccess(Option<LogIdOf<C>>),

    /// The first log id([`AppendEntriesRequest::prev_log_id`]) of the entries to send does not
    /// match on the remote target node.
    ///
    /// [`AppendEntriesRequest::prev_log_id`]: crate::raft::AppendEntriesRequest::prev_log_id
    Conflict,

    /// Seen a vote `v` that does not hold `mine_vote >= v`.
    /// And a leader's vote(committed vote) must be total order with other votes.
    /// Therefore, it has to be a higher vote: `mine_vote < v`
    HigherVote(VoteOf<C>),
}

impl<C> AppendEntriesResponse<C>
where C: RaftTypeConfig
{
    /// Returns true if the response indicates a successful replication.
    pub fn is_success(&self) -> bool {
        matches!(*self, AppendEntriesResponse::Success)
    }

    /// Returns the partial success log id if this is a `PartialSuccess` response.
    ///
    /// Returns `None` for `Success`, `Conflict`, or `HigherVote` responses.
    pub(crate) fn get_partial_success(&self) -> Option<&Option<LogIdOf<C>>> {
        match self {
            AppendEntriesResponse::PartialSuccess(log_id) => Some(log_id),
            _ => None,
        }
    }

    /// Returns true if the response indicates a log conflict.
    pub fn is_conflict(&self) -> bool {
        matches!(*self, AppendEntriesResponse::Conflict)
    }

    /// Convert this response to a stream append result.
    ///
    /// Arguments:
    /// - `prev_log_id`: The prev_log_id from the request, used for Conflict errors.
    /// - `last_log_id`: The last_log_id of the sent entries, used for Success.
    pub fn into_stream_result(
        self,
        prev_log_id: Option<LogIdOf<C>>,
        last_log_id: Option<LogIdOf<C>>,
    ) -> StreamAppendResult<C> {
        match self {
            AppendEntriesResponse::Success => Ok(last_log_id),
            AppendEntriesResponse::PartialSuccess(log_id) => Ok(log_id),
            AppendEntriesResponse::Conflict => Err(StreamAppendError::Conflict(prev_log_id.unwrap())),
            AppendEntriesResponse::HigherVote(vote) => Err(StreamAppendError::HigherVote(vote)),
        }
    }
}

impl<C> fmt::Display for AppendEntriesResponse<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppendEntriesResponse::Success => write!(f, "Success"),
            AppendEntriesResponse::PartialSuccess(m) => {
                write!(f, "PartialSuccess({})", m.display())
            }
            AppendEntriesResponse::HigherVote(vote) => write!(f, "Higher vote, {}", vote),
            AppendEntriesResponse::Conflict => write!(f, "Conflict"),
        }
    }
}
