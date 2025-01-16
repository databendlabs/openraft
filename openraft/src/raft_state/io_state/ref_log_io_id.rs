use crate::log_id::ref_log_id::RefLogId;
use crate::vote::committed::CommittedVote;
use crate::RaftTypeConfig;

/// A reference type of [`LogIOId`].
///
/// [`LogIOId`]: crate::raft_state::io_state::log_io_id::LogIOId
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
pub(crate) struct RefLogIOId<'a, C>
where C: RaftTypeConfig
{
    /// The id of the leader that performs the log io operation.
    pub(crate) committed_vote: &'a CommittedVote<C>,

    /// The last log id that has been flushed to storage.
    pub(crate) log_id: Option<RefLogId<'a, C>>,
}

impl<'a, C> RefLogIOId<'a, C>
where C: RaftTypeConfig
{
    pub(crate) fn new(committed_vote: &'a CommittedVote<C>, log_id: Option<RefLogId<'a, C>>) -> Self {
        Self { committed_vote, log_id }
    }

    pub(crate) fn committed_vote(&self) -> &'a CommittedVote<C> {
        self.committed_vote
    }

    pub(crate) fn log_id(&self) -> Option<RefLogId<'a, C>> {
        self.log_id
    }
}
