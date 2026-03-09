use crate::vote::RaftLeaderId;
use crate::vote::committed::CommittedVote;
use crate::vote::non_committed::UncommittedVote;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VoteStatus<LID>
where LID: RaftLeaderId
{
    Committed(CommittedVote<LID>),
    Pending(UncommittedVote<LID>),
}
