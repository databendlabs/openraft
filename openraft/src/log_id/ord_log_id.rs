use std::cmp::Ordering;
use std::fmt;

use crate::alias::CommittedLeaderIdOf;
use crate::alias::LogIdOf;
use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::RaftLogId;
use crate::RaftTypeConfig;

/// A wrapper type that implements [`PartialOrd`] and [`PartialEq`] based on the ordering key of the
/// inner type.
#[derive(Debug, Clone, Eq)]
pub(crate) struct OrdLogId<C>
where C: RaftTypeConfig
{
    pub(crate) inner: LogIdOf<C>,
}

impl<C> OrdLogId<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(inner: LogIdOf<C>) -> Self {
        Self { inner }
    }

    pub(crate) fn into_inner(self) -> LogIdOf<C> {
        self.inner
    }

    pub(crate) fn to_inner(&self) -> LogIdOf<C> {
        self.inner.clone()
    }

    pub(crate) fn inner(&self) -> &LogIdOf<C> {
        &self.inner
    }
}

impl<C> PartialEq for OrdLogId<C>
where C: RaftTypeConfig
{
    fn eq(&self, other: &Self) -> bool {
        self.inner.ord_by() == other.inner.ord_by()
    }
}

impl<C> PartialOrd for OrdLogId<C>
where C: RaftTypeConfig
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.inner.ord_by().partial_cmp(&other.inner.ord_by())
    }
}

impl<C> Ord for OrdLogId<C>
where C: RaftTypeConfig
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.inner.ord_by().cmp(&other.inner.ord_by())
    }
}

impl<C> fmt::Display for OrdLogId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl<C> RaftLogId<C> for OrdLogId<C>
where C: RaftTypeConfig
{
    fn new(leader_id: CommittedLeaderIdOf<C>, index: u64) -> Self {
        OrdLogId {
            inner: LogIdOf::<C>::new(leader_id, index),
        }
    }

    fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C> {
        self.inner.committed_leader_id()
    }

    fn index(&self) -> u64 {
        self.inner.index()
    }
}
