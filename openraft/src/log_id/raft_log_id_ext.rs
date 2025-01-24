use crate::log_id::ref_log_id::RefLogId;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::RaftLogId;
use crate::RaftTypeConfig;

pub(crate) trait RaftLogIdExt<C>
where
    C: RaftTypeConfig,
    Self: RaftLogId<C>,
{
    fn default() -> Self {
        Self::new(CommittedLeaderIdOf::<C>::default(), 0)
    }

    fn to_log_id(&self) -> LogIdOf<C> {
        self.ref_log_id().to_log_id()
    }

    fn ref_log_id(&self) -> RefLogId<'_, C> {
        RefLogId {
            leader_id: self.committed_leader_id(),
            index: self.index(),
        }
    }

    /// Returns the key used for comparing this value.
    fn ord_by(&self) -> RefLogId<'_, C> {
        self.ref_log_id()
    }
}

impl<C, T> RaftLogIdExt<C> for T
where
    C: RaftTypeConfig,
    T: RaftLogId<C>,
{
}
