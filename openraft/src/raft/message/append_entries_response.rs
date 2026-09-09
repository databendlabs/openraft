use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::raft::StreamAppendError;
use crate::raft::StreamAppendSuccess;
use crate::raft::stream_append::StreamAppendResult;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// The response to an `AppendEntriesRequest`.
///
/// [`RaftNetworkV2::append_entries`] returns this type only when received an RPC reply.
/// Otherwise, it should return [`RPCError`].
///
/// [`RPCError`]: crate::errors::RPCError
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
    /// The returned matching log id must be between
    /// [`AppendEntriesRequest::prev_log_id`] and the request's last log id, inclusive. A matching
    /// log id equal to the request's last log id is normalized to a full success by the stream
    /// adapter. If no RPC reply is received, [`RaftNetworkV2::append_entries`] must return an
    /// [`RPCError`] to inform Openraft that the first log id may not match on the remote target
    /// node.
    ///
    /// [`RPCError`]: crate::errors::RPCError
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

    /// Returns true if the response indicates a log conflict.
    pub fn is_conflict(&self) -> bool {
        matches!(*self, AppendEntriesResponse::Conflict)
    }

    /// Convert this response to a stream append result.
    ///
    /// Arguments:
    /// - `prev_log_id`: The prev_log_id from the request, used for Conflict errors.
    /// - `last_log_id`: The last_log_id of the sent entries, used for Success and for normalizing a
    ///   complete PartialSuccess response.
    #[since(version = "0.10.0", change = "stream success distinguishes full and partial")]
    pub fn into_stream_result(
        self,
        prev_log_id: Option<LogIdOf<C>>,
        last_log_id: Option<LogIdOf<C>>,
    ) -> StreamAppendResult<C> {
        match self {
            AppendEntriesResponse::Success => Ok(StreamAppendSuccess::Full(last_log_id)),
            AppendEntriesResponse::PartialSuccess(log_id) if log_id == last_log_id => {
                Ok(StreamAppendSuccess::Full(log_id))
            }
            AppendEntriesResponse::PartialSuccess(log_id) => {
                debug_assert!(prev_log_id <= log_id && log_id < last_log_id);
                Ok(StreamAppendSuccess::Partial(log_id))
            }
            AppendEntriesResponse::Conflict => Err(StreamAppendError::Conflict(prev_log_id.unwrap())),
            AppendEntriesResponse::HigherVote(vote) => Err(StreamAppendError::HigherVote(vote)),
        }
    }
}

impl<C> From<StreamAppendResult<C>> for AppendEntriesResponse<C>
where C: RaftTypeConfig
{
    fn from(r: StreamAppendResult<C>) -> Self {
        match r {
            Ok(StreamAppendSuccess::Full(_)) => AppendEntriesResponse::Success,
            Ok(StreamAppendSuccess::Partial(log_id)) => AppendEntriesResponse::PartialSuccess(log_id),
            Err(StreamAppendError::Conflict(_)) => AppendEntriesResponse::Conflict,
            Err(StreamAppendError::HigherVote(v)) => AppendEntriesResponse::HigherVote(v),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;

    #[test]
    fn test_into_stream_result_preserves_success_kind() {
        assert_eq!(
            AppendEntriesResponse::<UTConfig>::Success.into_stream_result(None, Some(log_id(1, 1, 10))),
            Ok(StreamAppendSuccess::Full(Some(log_id(1, 1, 10))))
        );

        assert_eq!(
            AppendEntriesResponse::<UTConfig>::PartialSuccess(Some(log_id(1, 1, 8)))
                .into_stream_result(Some(log_id(1, 1, 5)), Some(log_id(1, 1, 10))),
            Ok(StreamAppendSuccess::Partial(Some(log_id(1, 1, 8))))
        );

        assert_eq!(
            AppendEntriesResponse::<UTConfig>::PartialSuccess(Some(log_id(1, 1, 10)))
                .into_stream_result(Some(log_id(1, 1, 5)), Some(log_id(1, 1, 10))),
            Ok(StreamAppendSuccess::Full(Some(log_id(1, 1, 10))))
        );

        assert_eq!(
            AppendEntriesResponse::<UTConfig>::PartialSuccess(None).into_stream_result(None, Some(log_id(1, 1, 10))),
            Ok(StreamAppendSuccess::Partial(None))
        );
    }
}
