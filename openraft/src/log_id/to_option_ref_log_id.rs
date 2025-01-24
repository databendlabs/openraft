use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::log_id::ref_log_id::RefLogId;
use crate::RaftLogId;
use crate::RaftTypeConfig;

/// Convert `Option<&impl RaftLogId>` to `Option<RefLogId>`.
pub(crate) trait ToOptionRefLogId<'l, C>
where C: RaftTypeConfig
{
    fn to_ref(self) -> Option<RefLogId<'l, C>>;
}

impl<'l, C, T> ToOptionRefLogId<'l, C> for Option<&'l T>
where
    C: RaftTypeConfig,
    T: RaftLogId<C>,
{
    fn to_ref(self) -> Option<RefLogId<'l, C>> {
        self.map(|x| x.to_ref())
    }
}
