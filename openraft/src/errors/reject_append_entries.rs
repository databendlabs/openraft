use peel_off::Peel;

use crate::RaftTypeConfig;
use crate::errors::ConflictingLogId;
use crate::errors::RejectLeadership;
use crate::raft::AppendEntriesResponse;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RejectAppendEntries<C: RaftTypeConfig> {
    #[error("reject AppendEntries by a greater vote: {0}")]
    RejectLeadership(RejectLeadership<C>),

    #[error("reject AppendEntries due to conflicting log id: {0}")]
    ConflictingLogId(ConflictingLogId<C>),
}

impl<C: RaftTypeConfig> From<RejectLeadership<C>> for RejectAppendEntries<C> {
    fn from(r: RejectLeadership<C>) -> Self {
        RejectAppendEntries::RejectLeadership(r)
    }
}

impl<C: RaftTypeConfig> From<Result<(), RejectAppendEntries<C>>> for AppendEntriesResponse<C> {
    fn from(r: Result<(), RejectAppendEntries<C>>) -> Self {
        match r {
            Ok(_) => AppendEntriesResponse::Success,
            Err(e) => match e {
                RejectAppendEntries::RejectLeadership(r) => match r {
                    RejectLeadership::ByVote(v) => AppendEntriesResponse::HigherVote(v),
                    RejectLeadership::ByLastLogId(_) => {
                        unreachable!("the leader should always has a greater last log id")
                    }
                },
                RejectAppendEntries::ConflictingLogId(_) => AppendEntriesResponse::Conflict,
            },
        }
    }
}

/// Peel off `RejectVoteRequest`, leaving `ConflictingLogId` as the residual.
impl<C: RaftTypeConfig> Peel for RejectAppendEntries<C> {
    type Peeled = RejectLeadership<C>;
    type Residual = ConflictingLogId<C>;

    fn peel(self) -> Result<ConflictingLogId<C>, RejectLeadership<C>> {
        match self {
            RejectAppendEntries::RejectLeadership(e) => Err(e),
            RejectAppendEntries::ConflictingLogId(e) => Ok(e),
        }
    }
}
