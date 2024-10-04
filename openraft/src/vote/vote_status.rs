use crate::vote::CommittedVote;
use crate::vote::NonCommittedVote;
use crate::RaftTypeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VoteStatus<C: RaftTypeConfig> {
    Committed(CommittedVote<C>),
    Pending(NonCommittedVote<C>),
}
