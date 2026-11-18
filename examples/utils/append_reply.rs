use openraft::RaftTypeConfig;
use openraft::raft::AppendError;
use openraft::raft::AppendResult;
use openraft::raft::ConflictHint;
use openraft::type_config::alias::LogIdOf;
use openraft::type_config::alias::VoteOf;
use serde::Deserialize;
use serde::Serialize;

/// An [`AppendResult`] in a serializable wire format, for the reply to an AppendEntries request.
///
/// [`AppendResult`] does not implement serde, so the application defines how it travels. This
/// format keeps the log id the follower accepted and its conflict hint, which lets the leader skip
/// part of its binary search after a conflict.
///
/// The examples share this file through `#[path]`, and each one encodes the type with its own
/// codec.
///
/// Changing this type changes the bytes on the wire. A deployment that upgrades its nodes one at
/// a time must version this format, so that every node still understands the replies of a node
/// that runs the previous version.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
#[serde(bound = "")]
pub enum AppendReply<C>
where C: RaftTypeConfig
{
    /// The follower accepted the entries up to this log id.
    Accepted(Option<LogIdOf<C>>),

    /// The follower's log does not hold the request's `prev_log_id`.
    Conflict {
        /// The `prev_log_id` of the rejected request.
        log_id: LogIdOf<C>,

        /// The log ids of the follower's conflict hint, if it sent one.
        ///
        /// An empty list is a hint of its own: the follower's log is empty.
        hint: Option<Vec<LogIdOf<C>>>,
    },

    /// The follower has a higher vote than the leader.
    HigherVote(VoteOf<C>),
}

impl<C> From<AppendResult<C>> for AppendReply<C>
where C: RaftTypeConfig
{
    fn from(result: AppendResult<C>) -> Self {
        match result {
            Ok(log_id) => AppendReply::Accepted(log_id),
            Err(AppendError::Conflict { log_id, hint }) => AppendReply::Conflict {
                log_id,
                hint: hint.map(|hint| hint.log_ids),
            },
            Err(AppendError::HigherVote(vote)) => AppendReply::HigherVote(vote),
        }
    }
}

impl<C> From<AppendReply<C>> for AppendResult<C>
where C: RaftTypeConfig
{
    fn from(reply: AppendReply<C>) -> Self {
        match reply {
            AppendReply::Accepted(log_id) => Ok(log_id),
            AppendReply::Conflict { log_id, hint } => Err(AppendError::Conflict {
                log_id,
                hint: hint.map(ConflictHint::new),
            }),
            AppendReply::HigherVote(vote) => Err(AppendError::HigherVote(vote)),
        }
    }
}
