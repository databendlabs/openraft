use std::fmt;

use openraft_macros::since;
use peel_off::Peel;

use crate::RaftTypeConfig;
use crate::errors::ConflictingLogId;
use crate::errors::RejectVote;
use crate::type_config::alias::VoteOf;

/// Error type for stream append entries.
///
/// When this error is returned, the stream is terminated.
#[since(version = "0.10.0", change = "Conflict carries follower conflict details")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum StreamAppendError<C>
where C: RaftTypeConfig
{
    /// The follower's log does not match the leader's `prev_log_id`.
    Conflict(ConflictingLogId<C>),

    /// The follower has a higher vote than the sender's.
    HigherVote(VoteOf<C>),
}

impl<C> fmt::Display for StreamAppendError<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamAppendError::Conflict(conflict) => {
                write!(f, "Conflict({})", conflict)
            }
            StreamAppendError::HigherVote(vote) => {
                write!(f, "HigherVote({})", vote)
            }
        }
    }
}

/// Peel off `RejectVote`, leaving `ConflictingLogId` as the residual.
impl<C: RaftTypeConfig> Peel for StreamAppendError<C> {
    type Peeled = RejectVote<C>;
    type Residual = ConflictingLogId<C>;

    fn peel(self) -> Result<ConflictingLogId<C>, RejectVote<C>> {
        match self {
            StreamAppendError::HigherVote(vote) => Err(RejectVote { higher: vote }),
            StreamAppendError::Conflict(conflict) => Ok(conflict),
        }
    }
}
