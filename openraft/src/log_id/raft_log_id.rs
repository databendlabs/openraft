use std::fmt;

use crate::RaftTypeConfig;
use crate::type_config::alias::CommittedLeaderIdOf;

/// Log id is the globally unique identifier of a log entry.
///
/// Equal log id means the same log entry.
pub(crate) trait RaftLogId<C>
where
    C: RaftTypeConfig,
    Self: Eq + Clone + fmt::Debug + fmt::Display,
{
    /// Creates a log id proposed by a committed leader `leader_id` at the given index.
    // This is only used internally
    #[allow(dead_code)]
    fn new(leader_id: CommittedLeaderIdOf<C>, index: u64) -> Self;

    /// Returns a reference to the leader id that proposed this log id.  
    ///
    /// When a `LeaderId` is committed, some of its data can be discarded.
    /// For example, a leader id in standard raft is `(term, node_id)`, but a log id does not have
    /// to store the `node_id`, because in standard raft there is at most one leader that can be
    /// established.
    fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C>;

    /// Returns the index of the log id.
    fn index(&self) -> u64;
}

impl<C, T> RaftLogId<C> for &T
where
    C: RaftTypeConfig,
    T: RaftLogId<C>,
{
    fn new(_leader_id: CommittedLeaderIdOf<C>, _index: u64) -> Self {
        unreachable!("This method should not be called on a reference.")
    }

    fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C> {
        T::committed_leader_id(self)
    }

    fn index(&self) -> u64 {
        T::index(self)
    }
}
