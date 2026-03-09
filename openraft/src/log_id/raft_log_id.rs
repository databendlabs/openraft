use std::fmt;

use openraft_macros::since;

use crate::vote::leader_id::raft_committed_leader_id::RaftCommittedLeaderId;

/// Log id is the globally unique identifier of a log entry.
///
/// Equal log id means the same log entry.
#[since(
    version = "0.10.0",
    change = "removed `C: RaftTypeConfig` generic parameter, added `CommittedLeaderId` associated type"
)]
pub trait RaftLogId
where Self: Eq + Clone + fmt::Debug
{
    /// The committed leader ID type used by this log ID.
    #[since(version = "0.10.0")]
    type CommittedLeaderId: RaftCommittedLeaderId;

    /// Creates a log id proposed by a committed leader `leader_id` at the given index.
    // This is only used internally
    #[allow(dead_code)]
    fn new(leader_id: Self::CommittedLeaderId, index: u64) -> Self;

    /// Returns a reference to the leader id that proposed this log id.
    ///
    /// When a `LeaderId` is committed, some of its data can be discarded.
    /// For example, a leader id in standard raft is `(term, node_id)`, but a log id does not have
    /// to store the `node_id`, because in standard raft there is at most one leader that can be
    /// established.
    fn committed_leader_id(&self) -> &Self::CommittedLeaderId;

    /// Returns the index of the log id.
    fn index(&self) -> u64;

    /// Converts this log ID into another type that implements [`RaftLogId`].
    fn to_type<T>(&self) -> T
    where T: RaftLogId<CommittedLeaderId = Self::CommittedLeaderId> {
        T::new(self.committed_leader_id().clone(), self.index())
    }

    /// Returns the parts of this log ID as a tuple of (committed_leader_id, index).
    fn log_id_parts(&self) -> (&Self::CommittedLeaderId, u64) {
        (self.committed_leader_id(), self.index())
    }
}

impl<T> RaftLogId for &T
where T: RaftLogId
{
    type CommittedLeaderId = T::CommittedLeaderId;

    fn new(_leader_id: Self::CommittedLeaderId, _index: u64) -> Self {
        unreachable!("This method should not be called on a reference.")
    }

    fn committed_leader_id(&self) -> &Self::CommittedLeaderId {
        T::committed_leader_id(self)
    }

    fn index(&self) -> u64 {
        T::index(self)
    }
}
