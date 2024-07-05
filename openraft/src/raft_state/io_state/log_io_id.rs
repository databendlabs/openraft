use std::fmt;

use crate::display_ext::DisplayOptionExt;
use crate::CommittedLeaderId;
use crate::LogId;
use crate::RaftTypeConfig;

/// A monotonic increasing id for log io operation.
///
/// The leader could be a local leader that appends entries to the local log store,
/// or a remote leader that replicates entries to this follower.
///
/// It is monotonic increasing because:
/// - Leader id increase monotonically in the entire cluster.
/// - Leader propose or replicate log entries in order.
#[derive(Debug, Clone, Copy)]
#[derive(Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct LogIOId<C>
where C: RaftTypeConfig
{
    /// The id of the leader that performs the log io operation.
    pub(crate) committed_leader_id: CommittedLeaderId<C::NodeId>,

    /// The last log id that has been flushed to storage.
    pub(crate) log_id: Option<LogId<C::NodeId>>,
}

impl<C> fmt::Display for LogIOId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "by_leader({}):{}", self.committed_leader_id, self.log_id.display())
    }
}

impl<C> LogIOId<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(
        committed_leader_id: impl Into<CommittedLeaderId<C::NodeId>>,
        log_id: Option<LogId<C::NodeId>>,
    ) -> Self {
        Self {
            committed_leader_id: committed_leader_id.into(),
            log_id,
        }
    }
}
