use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::CommittedLeaderIdOf;

/// Optional behavior of a client write.
///
/// The default option proposes the write without conditions. Use [`with_leader()`] to make the
/// write conditional on the current leader.
///
/// [`with_leader()`]: WriteOption::with_leader
#[since(version = "0.10.1")]
#[derive(Debug, Clone, Default)]
#[derive(PartialEq, Eq)]
pub struct WriteOption<C>
where C: RaftTypeConfig
{
    /// The committed leader that must still be the established leader when the write is
    /// proposed; `None` accepts any leader.
    pub(crate) expected_leader: Option<CommittedLeaderIdOf<C>>,
}

impl<C> WriteOption<C>
where C: RaftTypeConfig
{
    /// Propose the write only if the current committed leader is `expected_leader`.
    ///
    /// If it is not, the write fails with [`ClientWriteError::ForwardToLeader`] and nothing is
    /// appended. See [`WriteRequest::with_leader()`] for what a leader id contains under each
    /// leader id type.
    ///
    /// [`ClientWriteError::ForwardToLeader`]: crate::errors::ClientWriteError::ForwardToLeader
    /// [`WriteRequest::with_leader()`]: crate::raft::WriteRequest::with_leader
    #[since(version = "0.10.1")]
    pub fn with_leader(mut self, expected_leader: impl Into<CommittedLeaderIdOf<C>>) -> Self {
        self.expected_leader = Some(expected_leader.into());
        self
    }

    /// Return whether a write with `other` can be merged into a write with this option.
    ///
    /// RaftCore merges consecutive client writes into one batch, and the batch is proposed with
    /// this option. A write with `other` may join the batch only if proposing it with this option
    /// behaves the same as proposing it with `other`.
    pub(crate) fn is_mergeable_with(&self, other: &Self) -> bool {
        self.expected_leader == other.expected_leader
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;

    type C = UTConfig<()>;

    fn committed_leader_id(term: u64, node_id: u64) -> CommittedLeaderIdOf<C> {
        *log_id(term, node_id, 0).committed_leader_id()
    }

    #[test]
    fn test_is_mergeable_with() {
        let any = WriteOption::<C>::default();
        let leader1 = WriteOption::<C>::default().with_leader(committed_leader_id(1, 1));
        let leader2 = WriteOption::<C>::default().with_leader(committed_leader_id(2, 1));

        assert!(any.is_mergeable_with(&WriteOption::default()));
        assert!(leader1.is_mergeable_with(&WriteOption::default().with_leader(committed_leader_id(1, 1))));

        assert!(!any.is_mergeable_with(&leader1));
        assert!(!leader1.is_mergeable_with(&any));
        assert!(!leader1.is_mergeable_with(&leader2));
    }
}
