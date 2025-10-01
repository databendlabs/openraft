use std::fmt::Display;
use std::fmt::Formatter;

use crate::LogId;
use crate::RaftTypeConfig;
use crate::log_id::raft_log_id::RaftLogId;
use crate::type_config::alias::CommittedLeaderIdOf;

/// A reference to a log id, combining a reference to a committed leader ID and an index.
/// Committed leader ID is the `term` in standard Raft.
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
    /// Create a new reference log id.
    pub(crate) fn new(leader_id: &'l CommittedLeaderIdOf<C>, index: u64) -> Self {
        RefLogId { leader_id, index }
    }

    /// Return the committed leader ID of this log id, which is `term` in standard Raft.
    pub(crate) fn committed_leader_id(&self) -> &'l CommittedLeaderIdOf<C> {
        self.leader_id
    }

    /// Return the index of this log id.
    pub(crate) fn index(&self) -> u64 {
        self.index
    }

    /// Convert this reference type to an owned log id.
    pub(crate) fn into_log_id(self) -> LogId<C> {
        LogId::<C>::new(self.leader_id.clone(), self.index)
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
