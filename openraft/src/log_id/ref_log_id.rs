use std::fmt::Display;
use std::fmt::Formatter;

use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::RaftLogId;
use crate::RaftTypeConfig;

/// A reference to a log id, combining a reference to a committed leader ID and an index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RefLogId<'k, C>
where C: RaftTypeConfig
{
    pub(crate) leader_id: &'k CommittedLeaderIdOf<C>,
    pub(crate) index: u64,
}

impl<'l, C> RefLogId<'l, C>
where C: RaftTypeConfig
{
    pub(crate) fn new(leader_id: &'l CommittedLeaderIdOf<C>, index: u64) -> Self {
        RefLogId { leader_id, index }
    }

    pub(crate) fn committed_leader_id(&self) -> &'l CommittedLeaderIdOf<C> {
        self.leader_id
    }

    pub(crate) fn index(&self) -> u64 {
        self.index
    }

    pub(crate) fn to_log_id(&self) -> LogIdOf<C> {
        LogIdOf::<C>::new(self.leader_id.clone(), self.index)
    }
}

impl<C> Display for RefLogId<'_, C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.committed_leader_id(), self.index())
    }
}

impl<C> RaftLogId<C> for RefLogId<'_, C>
where C: RaftTypeConfig
{
    fn new(_leader_id: CommittedLeaderIdOf<C>, _index: u64) -> Self {
        unreachable!("RefLogId does not own the leader id, so it cannot be created from it.")
    }

    fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C> {
        self.leader_id
    }

    fn index(&self) -> u64 {
        self.index
    }
}
