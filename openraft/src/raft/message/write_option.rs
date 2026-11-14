use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::CommittedLeaderIdOf;

/// Optional behavior of a client write.
///
/// The default option proposes the write without conditions. Use [`expect_leader()`] to create an
/// option that makes the write conditional on the current leader.
///
/// [`expect_leader()`]: WriteOption::expect_leader
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
    /// Create an option that proposes the write only if the current committed leader is
    /// `expected_leader`.
    ///
    /// The same as `WriteOption::default().with_leader(expected_leader)`; see
    /// [`with_leader()`](Self::with_leader).
    ///
    /// ```ignore
    /// let term = raft.as_leader()?.term();
    /// raft.client_write_many([data1, data2], WriteOption::expect_leader(term)).await?;
    /// ```
    #[since(version = "0.10.1")]
    pub fn expect_leader(expected_leader: impl Into<CommittedLeaderIdOf<C>>) -> Self {
        Self::default().with_leader(expected_leader)
    }

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

    /// Merge this option with `other` into the option for a batch holding writes of both.
    ///
    /// RaftCore merges consecutive client writes into one batch and proposes the batch with a
    /// single option. Returns that option, or `None` if the writes cannot share one: a merged
    /// option must make each write behave as its own option asks.
    ///
    /// Two expected leaders merge only when they are equal. Merging a write that accepts any leader
    /// with one that expects a specific leader would add or drop that condition for one of them.
    pub(crate) fn merge(&self, other: &Self) -> Option<Self> {
        if self.expected_leader != other.expected_leader {
            return None;
        }

        Some(Self {
            expected_leader: self.expected_leader.clone(),
        })
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
    fn test_merge() {
        let any = WriteOption::<C>::default();
        let leader1 = WriteOption::<C>::default().with_leader(committed_leader_id(1, 1));
        let leader2 = WriteOption::<C>::default().with_leader(committed_leader_id(2, 1));

        assert_eq!(Some(any.clone()), any.merge(&WriteOption::default()));
        assert_eq!(
            Some(leader1.clone()),
            leader1.merge(&WriteOption::default().with_leader(committed_leader_id(1, 1)))
        );

        assert_eq!(None, any.merge(&leader1));
        assert_eq!(None, leader1.merge(&any));
        assert_eq!(None, leader1.merge(&leader2));
    }

    #[test]
    fn test_expect_leader() {
        let leader_id = committed_leader_id(1, 1);

        assert_eq!(
            WriteOption::<C>::default().with_leader(leader_id),
            WriteOption::<C>::expect_leader(leader_id)
        );
        assert_eq!(
            Some(leader_id),
            WriteOption::<C>::expect_leader(leader_id).expected_leader
        );
    }
}
