use std::cmp::Ordering;
use std::fmt;

use crate::display_ext::DisplayOptionExt;
use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::raft_state::io_state::ref_log_io_id::RefLogIOId;
use crate::type_config::alias::LogIdOf;
use crate::vote::committed::CommittedVote;
use crate::RaftTypeConfig;

/// A monotonic increasing id for log append io operation.
///
/// The last appended [`LogId`] itself is not monotonic,
/// For example, Leader-1 appends log `[2,3]` and then Leader-2 truncate log `[2,3]` then append log
/// `[2]` But `(LeaderId, LogId)` is monotonic increasing.
///
/// The leader could be a local leader that appends entries to the local log store,
/// or a remote leader that replicates entries to this follower.
///
/// It is monotonic increasing because:
/// - Leader id increase monotonically in the entire cluster.
/// - Leader propose or replicate log entries in order.
///
/// See: [LogId Appended Multiple
/// Times](crate::docs::protocol::replication::log_replication#logid-appended-multiple-times).
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
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

impl<C> PartialOrd for LogIOId<C>
where C: RaftTypeConfig
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        PartialOrd::partial_cmp(&self.as_ref_log_io_id(), &other.as_ref_log_io_id())
    }
}

impl<C> Ord for LogIOId<C>
where C: RaftTypeConfig
{
    fn cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(&self.as_ref_log_io_id(), &other.as_ref_log_io_id())
    }
}

impl<C> LogIOId<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(committed_vote: CommittedVote<C>, log_id: Option<LogIdOf<C>>) -> Self {
        Self { committed_vote, log_id }
    }

    pub(crate) fn as_ref_log_io_id(&self) -> RefLogIOId<'_, C> {
        RefLogIOId::new(&self.committed_vote, self.log_id.as_ref().map(|x| x.ref_log_id()))
    }
}
