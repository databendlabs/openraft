use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// Error type for stream append entries.
///
/// When this error is returned, the stream is terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum StreamAppendError<C: RaftTypeConfig> {
    /// Log conflict at the given prev_log_id.
    ///
    /// The follower's log at this position does not match the leader's.
    Conflict(Option<LogIdOf<C>>),

    /// Seen a higher vote from another leader.
    HigherVote(VoteOf<C>),
}

impl<C> fmt::Display for StreamAppendError<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamAppendError::Conflict(log_id) => {
                write!(f, "Conflict({})", log_id.display())
            }
            StreamAppendError::HigherVote(vote) => {
                write!(f, "HigherVote({})", vote)
            }
        }
    }
}
