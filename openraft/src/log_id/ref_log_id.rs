use std::fmt::Display;
use std::fmt::Formatter;

use crate::LogId;
use crate::log_id::raft_log_id::RaftLogId;
use crate::vote::RaftCommittedLeaderId;

/// A reference to a log id, combining a reference to a committed leader ID and an index.
/// Committed leader ID is the `term` in standard Raft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RefLogId<'k, CLID>
where CLID: RaftCommittedLeaderId
{
    pub(crate) leader_id: &'k CLID,
    pub(crate) index: u64,
}

impl<'l, CLID> RefLogId<'l, CLID>
where CLID: RaftCommittedLeaderId
{
    /// Create a new reference log id.
    pub(crate) fn new(leader_id: &'l CLID, index: u64) -> Self {
        RefLogId { leader_id, index }
    }

    /// Return the committed leader ID of this log id, which is `term` in standard Raft.
    pub(crate) fn committed_leader_id(&self) -> &'l CLID {
        self.leader_id
    }

    /// Return the index of this log id.
    pub(crate) fn index(&self) -> u64 {
        self.index
    }

    /// Convert this reference type to an owned log id.
    pub(crate) fn into_log_id(self) -> LogId<CLID> {
        LogId::<CLID>::new(self.leader_id.clone(), self.index)
    }
}

impl<CLID> Display for RefLogId<'_, CLID>
where CLID: RaftCommittedLeaderId
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.committed_leader_id(), self.index())
    }
}

impl<CLID> RaftLogId for RefLogId<'_, CLID>
where CLID: RaftCommittedLeaderId
{
    type CommittedLeaderId = CLID;

    fn new(_leader_id: CLID, _index: u64) -> Self {
        unreachable!("RefLogId does not own the leader id, so it cannot be created from it.")
    }

    fn committed_leader_id(&self) -> &CLID {
        self.leader_id
    }

    fn index(&self) -> u64 {
        self.index
    }
}
