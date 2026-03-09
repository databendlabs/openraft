use crate::LogId;
use crate::log_id::raft_log_id::RaftLogId;
use crate::log_id::ref_log_id::RefLogId;

pub(crate) trait RaftLogIdExt: RaftLogId {
    /// Creates a new owned [`LogId`] from this log ID implementation.
    ///
    /// [`LogId`]: crate::log_id::LogId
    fn to_log_id(&self) -> LogId<Self::CommittedLeaderId> {
        self.to_ref().into_log_id()
    }

    /// Creates a reference view of this log ID implementation via a [`RefLogId`].
    fn to_ref(&self) -> RefLogId<'_, Self::CommittedLeaderId> {
        RefLogId {
            leader_id: self.committed_leader_id(),
            index: self.index(),
        }
    }
}

impl<T> RaftLogIdExt for T where T: RaftLogId {}
