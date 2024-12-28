use crate::vote::committed::CommittedVote;
use crate::vote::non_committed::NonCommittedVote;
use crate::RaftTypeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VoteStatus<C: RaftTypeConfig> {
    Committed(CommittedVote<C>),
    Pending(NonCommittedVote<C>),
}
