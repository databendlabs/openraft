use crate::RaftTypeConfig;
use crate::vote::committed::CommittedVote;
use crate::vote::non_committed::NonCommittedVote;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VoteStatus<C: RaftTypeConfig> {
    Committed(CommittedVote<C>),
    Pending(NonCommittedVote<C>),
}
