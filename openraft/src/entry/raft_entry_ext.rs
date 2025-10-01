use crate::RaftTypeConfig;
use crate::entry::RaftEntry;
use crate::log_id::ref_log_id::RefLogId;

pub(crate) trait RaftEntryExt<C>: RaftEntry<C>
where C: RaftTypeConfig
{
    /// Returns a lightweight [`RefLogId`] that contains the log id information.
    fn ref_log_id(&self) -> RefLogId<'_, C> {
        let (leader_id, index) = self.log_id_parts();
        RefLogId::new(leader_id, index)
    }
}

impl<C, T> RaftEntryExt<C> for T
where
    C: RaftTypeConfig,
    T: RaftEntry<C>,
{
}
