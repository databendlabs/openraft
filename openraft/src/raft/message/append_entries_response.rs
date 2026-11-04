use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::raft::AppendError;
use crate::raft::AppendResult;
use crate::raft::StreamAppendError;
use crate::raft::stream_append::StreamAppendResult;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// The response to an `AppendEntriesRequest`, in a serializable format without the conflict hint.
///
/// [`Raft::append_entries()`] and [`RaftNetworkV2::append_entries()`] use [`AppendResult`]. A
/// transport that keeps sending this format converts with `AppendEntriesResponse::from(result)`
/// on the follower, and with [`Self::into_append_result()`] on the leader.
///
/// [`Raft::append_entries()`]: crate::raft::Raft::append_entries
/// [`RaftNetworkV2::append_entries()`]: crate::network::RaftNetworkV2::append_entries
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum AppendEntriesResponse<C: RaftTypeConfig> {
    /// Successfully replicated all log entries to the target node.
    Success,

    /// Successfully sent the first portion of log entries.
    ///
    /// A transport can report a partial success.
    /// For example, it tries to send log entries `[1-2..3-10]`, the application is allowed to send
    /// just `[1-2..1-3]` and return `PartialSuccess(1-3)`
    ///
    /// ### Caution
    ///
    /// The returned matching log id must be **greater than or equal to** the first log
    /// id([`AppendEntriesRequest::prev_log_id`]) of the entries to send. If no RPC reply is
    /// received, the transport must return an [`RPCError`] to inform
    /// Openraft that the first log id([`AppendEntriesRequest::prev_log_id`]) may not match on
    /// the remote target node.
    ///
    /// [`RPCError`]: crate::errors::RPCError
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

    /// Convert this response to the [`AppendResult`] the leader handles.
    ///
    /// `prev_log_id` and `last_log_id` must describe the request the follower received: a
    /// transport that sent only a prefix of the entries passes the last log id of that prefix.
    /// This format carries no conflict hint.
    #[since(version = "0.10.0")]
    pub fn into_append_result(
        self,
        prev_log_id: Option<LogIdOf<C>>,
        last_log_id: Option<LogIdOf<C>>,
    ) -> AppendResult<C> {
        let result = self.into_stream_result(prev_log_id, last_log_id);
        result.map_err(AppendError::from)
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

impl<C: RaftTypeConfig> From<StreamAppendResult<C>> for AppendEntriesResponse<C> {
    fn from(r: StreamAppendResult<C>) -> Self {
        match r {
            Ok(_) => AppendEntriesResponse::Success,
            Err(StreamAppendError::Conflict(_)) => AppendEntriesResponse::Conflict,
            Err(StreamAppendError::HigherVote(v)) => AppendEntriesResponse::HigherVote(v),
        }
    }
}

impl<C> From<AppendResult<C>> for AppendEntriesResponse<C>
where C: RaftTypeConfig
{
    /// Convert a follower's reply to this format, which drops the acknowledged log ID and the
    /// conflict hint.
    fn from(result: AppendResult<C>) -> Self {
        match result {
            Ok(_) => AppendEntriesResponse::Success,
            Err(AppendError::Conflict { .. }) => AppendEntriesResponse::Conflict,
            Err(AppendError::HigherVote(vote)) => AppendEntriesResponse::HigherVote(vote),
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
    use crate::Vote;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::raft::AppendEntriesResponse;
    use crate::raft::AppendError;
    use crate::raft::AppendResult;

    #[test]
    fn into_append_result_takes_log_ids_from_the_sent_request() {
        let prev = Some(log_id(1, 1, 3));
        let last = Some(log_id(1, 1, 5));
        let partial = Some(log_id(1, 1, 4));
        let higher = Vote::new(2, 1);

        let cases: Vec<(AppendEntriesResponse<UTConfig>, AppendResult<UTConfig>)> = vec![
            (AppendEntriesResponse::Success, Ok(last)),
            (AppendEntriesResponse::PartialSuccess(partial), Ok(partial)),
            (
                AppendEntriesResponse::Conflict,
                Err(AppendError::Conflict {
                    log_id: log_id(1, 1, 3),
                    hint: None,
                }),
            ),
            (
                AppendEntriesResponse::HigherVote(higher),
                Err(AppendError::HigherVote(higher)),
            ),
        ];

        for (response, expected) in cases {
            let actual = response.into_append_result(prev, last);
            assert_eq!(expected, actual);
        }
    }
}
