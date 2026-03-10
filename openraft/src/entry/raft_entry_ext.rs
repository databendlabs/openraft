use crate::entry::RaftEntry;
use crate::log_id::ref_log_id::RefLogId;

pub(crate) trait RaftEntryExt: RaftEntry {
    /// Returns a lightweight [`RefLogId`] that contains the log id information.
    fn ref_log_id(&self) -> RefLogId<'_, Self::CommittedLeaderId> {
        let (leader_id, index) = self.log_id_parts();
        RefLogId::new(leader_id, index)
    }
}

impl<T> RaftEntryExt for T where T: RaftEntry {}
