use std::fmt;

use display_more::DisplaySliceExt;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::raft::ConflictHint;
use crate::raft::StreamAppendError;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// The outcome of one AppendEntries request: the last log ID the follower accepted, or why it
/// rejected the request.
#[since(version = "0.10.0")]
pub type AppendResult<C> = Result<Option<LogIdOf<C>>, AppendError<C>>;

/// Why a follower rejected an AppendEntries request.
///
/// [`Raft::stream_append()`] yields it on the follower, and [`NetStreamAppend::stream_append()`]
/// returns it to the leader.
///
/// This type does not implement serde. A transport defines its own wire format and converts to
/// and from it, so a change to this type cannot silently change the bytes on the wire. To keep the
/// existing format, convert to and from [`StreamAppendError`], which drops the conflict hint.
///
/// The enum is exhaustive on purpose: a variant or field added later fails to compile in each
/// transport's conversion, where the transport decides how to encode it.
///
/// [`Raft::stream_append()`]: crate::Raft::stream_append
/// [`NetStreamAppend::stream_append()`]: crate::network::NetStreamAppend::stream_append
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendError<C>
where C: RaftTypeConfig
{
    /// The follower's log does not hold the request's `prev_log_id`.
    Conflict {
        /// The `prev_log_id` of the rejected request.
        log_id: LogIdOf<C>,

        /// The follower's log after it truncated the conflicting entries, to guide the next probe.
        ///
        /// It may describe entries that are not flushed, so it acknowledges no replication.
        /// `None` if the transport does not carry hints.
        hint: Option<ConflictHint<C>>,
    },

    /// The follower has a higher vote than the sender's.
    HigherVote(VoteOf<C>),
}

impl<C> fmt::Display for AppendError<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppendError::Conflict { log_id, hint } => {
                write!(f, "Conflict({})", log_id)?;
                if let Some(hint) = hint {
                    write!(f, ", hint: {}", hint.log_ids.display())?;
                }
                Ok(())
            }
            AppendError::HigherVote(vote) => write!(f, "HigherVote({})", vote),
        }
    }
}

impl<C> From<StreamAppendError<C>> for AppendError<C>
where C: RaftTypeConfig
{
    /// Convert a rejection received in a format without conflict hints.
    fn from(error: StreamAppendError<C>) -> Self {
        match error {
            StreamAppendError::Conflict(log_id) => AppendError::Conflict { log_id, hint: None },
            StreamAppendError::HigherVote(vote) => AppendError::HigherVote(vote),
        }
    }
}

impl<C> From<AppendError<C>> for StreamAppendError<C>
where C: RaftTypeConfig
{
    /// Drop the conflict hint, to send the rejection in the [`StreamAppendError`] format.
    fn from(error: AppendError<C>) -> Self {
        match error {
            AppendError::Conflict { log_id, hint: _ } => StreamAppendError::Conflict(log_id),
            AppendError::HigherVote(vote) => StreamAppendError::HigherVote(vote),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Vote;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::raft::AppendError;
    use crate::raft::ConflictHint;
    use crate::raft::StreamAppendError;

    #[test]
    fn stream_append_error_converts_without_a_hint() {
        let conflict = StreamAppendError::<UTConfig>::Conflict(log_id(1, 1, 5));
        let expected = AppendError::Conflict {
            log_id: log_id(1, 1, 5),
            hint: None,
        };
        assert_eq!(expected, AppendError::from(conflict));

        let higher = StreamAppendError::<UTConfig>::HigherVote(Vote::new(2, 1));
        let expected = AppendError::HigherVote(Vote::new(2, 1));
        assert_eq!(expected, AppendError::from(higher));
    }

    #[test]
    fn conversion_to_stream_append_error_drops_the_hint() {
        let hint = ConflictHint::new(vec![log_id(1, 1, 3)]);
        let conflict = AppendError::<UTConfig>::Conflict {
            log_id: log_id(1, 1, 5),
            hint: Some(hint),
        };
        let expected = StreamAppendError::Conflict(log_id(1, 1, 5));
        assert_eq!(expected, StreamAppendError::from(conflict));
    }

    #[test]
    fn display_includes_the_hint() {
        let hint = ConflictHint::new(vec![log_id(1, 1, 3)]);
        let conflict = AppendError::<UTConfig>::Conflict {
            log_id: log_id(1, 1, 5),
            hint: Some(hint),
        };
        let without_hint = AppendError::<UTConfig>::Conflict {
            log_id: log_id(1, 1, 5),
            hint: None,
        };
        assert_eq!("Conflict(T1-N1.5), hint: [T1-N1.3]", conflict.to_string());
        assert_eq!("Conflict(T1-N1.5)", without_hint.to_string());
    }
}
