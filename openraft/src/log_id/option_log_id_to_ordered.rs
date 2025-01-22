use crate::log_id::ord_log_id::OrdLogId;
use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::RaftLogId;
use crate::RaftTypeConfig;

pub(crate) trait OptionLogIdToOrdered<C>
where C: RaftTypeConfig
{
    fn to_ordered(&self) -> Option<OrdLogId<C>>
    where Self: Sized {
        self.clone().into_ordered()
    }

    fn into_ordered(self) -> Option<OrdLogId<C>>
    where Self: Sized;
}

impl<C, T> OptionLogIdToOrdered<C> for Option<T>
where
    C: RaftTypeConfig<LogId = T>,
    T: RaftLogId<C>,
{
    fn into_ordered(self) -> Option<OrdLogId<C>>
    where Self: Sized {
        self.map(|x| x.into_ordered())
    }
}
