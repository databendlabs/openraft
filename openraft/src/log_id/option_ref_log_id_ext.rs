use crate::LogId;
use crate::log_id::ref_log_id::RefLogId;
use crate::vote::RaftCommittedLeaderId;

pub(crate) trait OptionRefLogIdExt<CLID>
where CLID: RaftCommittedLeaderId
{
    /// Creates a new owned [`LogId`] from the reference log ID.
    ///
    /// [`LogId`]: crate::log_id::LogId
    fn to_log_id(&self) -> Option<LogId<CLID>>;
}

impl<CLID> OptionRefLogIdExt<CLID> for Option<RefLogId<'_, CLID>>
where CLID: RaftCommittedLeaderId
{
    fn to_log_id(&self) -> Option<LogId<CLID>> {
        self.as_ref().map(|r| LogId::new(r.leader_id.clone(), r.index))
    }
}
