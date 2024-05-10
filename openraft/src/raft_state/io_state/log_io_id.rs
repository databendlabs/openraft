use std::fmt;

use crate::display_ext::DisplayOptionExt;
use crate::LeaderId;
use crate::LogId;
use crate::NodeId;

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
pub(crate) struct LogIOId<NID: NodeId> {
    /// The id of the leader that performs the log io operation.
    pub(crate) leader_id: LeaderId<NID>,

    /// The last log id that has been flushed to storage.
    pub(crate) log_id: Option<LogId<NID>>,
}

impl<NID: NodeId> fmt::Display for LogIOId<NID> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "by_leader({}):{}", self.leader_id, self.log_id.display())
    }
}

impl<NID: NodeId> LogIOId<NID> {
    pub(crate) fn new(leader_id: impl Into<LeaderId<NID>>, log_id: Option<LogId<NID>>) -> Self {
        Self {
            leader_id: leader_id.into(),
            log_id,
        }
    }
}
