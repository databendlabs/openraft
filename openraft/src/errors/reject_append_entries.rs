use peel_off::Peel;

use crate::RaftTypeConfig;
use crate::errors::ConflictingLogId;
use crate::errors::RejectVote;
use crate::raft::StreamAppendError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RejectAppendEntries<C: RaftTypeConfig> {
    #[error("reject AppendEntries by a greater vote: {0}")]
    RejectVote(RejectVote<C>),

    #[error("reject AppendEntries due to conflicting log id: {0}")]
    ConflictingLogId(ConflictingLogId<C>),
}

impl<C: RaftTypeConfig> From<RejectVote<C>> for RejectAppendEntries<C> {
    fn from(r: RejectVote<C>) -> Self {
        RejectAppendEntries::RejectVote(r)
    }
}

impl<C: RaftTypeConfig> From<RejectAppendEntries<C>> for StreamAppendError<C> {
    fn from(e: RejectAppendEntries<C>) -> Self {
        match e {
            RejectAppendEntries::RejectVote(r) => StreamAppendError::HigherVote(r.higher),
            RejectAppendEntries::ConflictingLogId(c) => StreamAppendError::Conflict(c.expect),
        }
    }
}

/// Peel off `RejectVote`, leaving `ConflictingLogId` as the residual.
impl<C: RaftTypeConfig> Peel for RejectAppendEntries<C> {
    type Peeled = RejectVote<C>;
    type Residual = ConflictingLogId<C>;

    fn peel(self) -> Result<ConflictingLogId<C>, RejectVote<C>> {
        match self {
            RejectAppendEntries::RejectVote(e) => Err(e),
            RejectAppendEntries::ConflictingLogId(e) => Ok(e),
        }
    }
}
