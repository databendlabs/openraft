use crate::log_id::ref_log_id::RefLogId;
use crate::type_config::alias::LogIdOf;
use crate::RaftLogId;
use crate::RaftTypeConfig;

pub(crate) trait RaftLogIdExt<C>
where
    C: RaftTypeConfig,
    Self: RaftLogId<C>,
{
    /// Creates a new owned [`LogId`] from this log ID implementation.
    fn to_log_id(&self) -> LogIdOf<C> {
        self.to_ref().to_log_id()
    }

    /// Creates a reference view of this log ID implementation via a [`RefLogId`].
    fn to_ref(&self) -> RefLogId<'_, C> {
        RefLogId {
            leader_id: self.committed_leader_id(),
            index: self.index(),
        }
    }
}

impl<C, T> RaftLogIdExt<C> for T
where
    C: RaftTypeConfig,
    T: RaftLogId<C>,
{
}
