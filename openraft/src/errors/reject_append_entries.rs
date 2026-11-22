use peel_off::Peel;

use crate::RaftTypeConfig;
use crate::errors::ConflictingLogId;
use crate::errors::RejectVote;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RejectAppendEntries<C>
where C: RaftTypeConfig
{
    #[error("reject AppendEntries by a greater vote: {0}")]
    RejectVote(RejectVote<C>),

    #[error("reject AppendEntries due to conflicting log id: {0}")]
    ConflictingLogId(ConflictingLogId<C>),
}

impl<C> From<RejectVote<C>> for RejectAppendEntries<C>
where C: RaftTypeConfig
{
    fn from(r: RejectVote<C>) -> Self {
        RejectAppendEntries::RejectVote(r)
    }
}

/// Peel off `RejectVote`, leaving `ConflictingLogId` as the residual.
impl<C> Peel for RejectAppendEntries<C>
where C: RaftTypeConfig
{
    type Peeled = RejectVote<C>;
    type Residual = ConflictingLogId<C>;

    fn peel(self) -> Result<ConflictingLogId<C>, RejectVote<C>> {
        match self {
            RejectAppendEntries::RejectVote(e) => Err(e),
            RejectAppendEntries::ConflictingLogId(e) => Ok(e),
        }
    }
}
