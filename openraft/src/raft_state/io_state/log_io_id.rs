use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::raft_state::io_state::IOId;
use crate::type_config::alias::LogIdOf;
use crate::vote::committed::CommittedVote;

/// Monotonic increasing identifier for log append I/O operations.
///
/// `LogIOId = (CommittedVote, LogId)` ensures monotonicity even when the same [`LogId`] is
/// truncated and appended multiple times by different leaders.
///
/// For a comprehensive explanation, see: [`LogIOId` documentation](crate::docs::data::log_io_id).
///
/// [`LogId`]: crate::log_id::LogId
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
pub(crate) struct LogIOId<C>
where C: RaftTypeConfig
{
    /// The id of the leader that performs the log io operation.
    pub(crate) committed_vote: CommittedVote<C>,

    /// The last log id that has been flushed to storage.
    pub(crate) log_id: Option<LogIdOf<C>>,
}

impl<C> fmt::Display for LogIOId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "by:{}, {}", self.committed_vote, self.log_id.display())
    }
}

impl<C> LogIOId<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(committed_vote: CommittedVote<C>, log_id: Option<LogIdOf<C>>) -> Self {
        Self { committed_vote, log_id }
    }

    #[allow(dead_code)]
    pub(crate) fn committed_vote(&self) -> &CommittedVote<C> {
        &self.committed_vote
    }

    pub(crate) fn to_committed_vote(&self) -> CommittedVote<C> {
        self.committed_vote.clone()
    }

    /// Return the last log id included in this io operation.
    pub(crate) fn last_log_id(&self) -> Option<&LogIdOf<C>> {
        self.log_id.as_ref()
    }

    pub(crate) fn to_io_id(&self) -> IOId<C> {
        IOId::Log(self.clone())
    }
}
