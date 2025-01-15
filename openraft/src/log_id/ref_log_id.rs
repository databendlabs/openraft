use std::fmt::Display;
use std::fmt::Formatter;

use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::RaftTypeConfig;

/// A reference to a log id, combining a reference to a committed leader ID and an index.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RefLogId<'k, C>
where C: RaftTypeConfig
{
    pub(crate) committed_leader_id: &'k CommittedLeaderIdOf<C>,
    pub(crate) index: u64,
}

impl<'l, C> RefLogId<'l, C>
where C: RaftTypeConfig
{
    pub(crate) fn new(committed_leader_id: &'l CommittedLeaderIdOf<C>, index: u64) -> Self {
        RefLogId {
            committed_leader_id,
            index,
        }
    }

    pub(crate) fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C> {
        self.committed_leader_id
    }

    pub(crate) fn index(&self) -> u64 {
        self.index
    }

    pub(crate) fn to_log_id(&self) -> LogIdOf<C> {
        LogIdOf::<C>::new(self.committed_leader_id.clone(), self.index)
    }
}

impl<C> Display for RefLogId<'_, C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.committed_leader_id(), self.index())
    }
}
