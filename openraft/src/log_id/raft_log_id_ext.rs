use crate::log_id::ord_log_id::OrdLogId;
use crate::log_id::ref_log_id::RefLogId;
use crate::type_config::alias::CommittedLeaderIdOf;
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

    fn as_ref_log_id(&self) -> RefLogId<'_, C> {
        RefLogId {
            leader_id: self.leader_id(),
            index: self.index(),
        }
    }

    /// Returns the key used for comparing this value.
    fn ord_by(&self) -> RefLogId<'_, C> {
        self.as_ref_log_id()
    }

    /// Returns a wrapped value that implements [`PartialOrd`] and [`PartialEq`] based on the
    /// ordering key.
    fn ordered(self) -> OrdLogId<C>
    where
        Self: Sized,
        C: RaftTypeConfig<LogId = Self>,
    {
        OrdLogId { inner: self }
    }
}

impl<C, T> RaftLogIdExt<C> for T
where
    C: RaftTypeConfig,
    T: RaftLogId<C>,
{
}
