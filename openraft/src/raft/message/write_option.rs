use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;

/// Optional behavior of a client write.
///
/// The default option proposes the write without conditions. Use [`expect_leader()`] to create an
/// option that makes the write conditional on the current leader, and [`expect_last_log_id()`]
/// for one conditional on the leader's last log id.
///
/// [`expect_leader()`]: WriteOption::expect_leader
/// [`expect_last_log_id()`]: WriteOption::expect_last_log_id
#[since(version = "0.10.2", change = "add an expected last log id")]
#[since(version = "0.10.1")]
#[derive(Debug, Clone, Default)]
#[derive(PartialEq, Eq)]
pub struct WriteOption<C>
where C: RaftTypeConfig
{
    /// The committed leader that must still be the established leader when the write is
    /// proposed; `None` accepts any leader.
    pub(crate) expected_leader: Option<CommittedLeaderIdOf<C>>,

    /// The log id that must be the leader's last log id right before the write is appended;
    /// `None` accepts any log.
    pub(crate) expected_last_log_id: Option<LogIdOf<C>>,
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
    /// ```no_run
    /// # use openraft::Raft;
    /// # use openraft::RaftTypeConfig;
    /// # use openraft::storage::RaftStateMachine;
    /// use openraft::raft::WriteOption;
    ///
    /// # async fn example<C, SM>(
    /// #     raft: &Raft<C, SM>,
    /// #     data1: C::D,
    /// #     data2: C::D,
    /// # ) -> Result<(), Box<dyn std::error::Error>>
    /// # where
    /// #     C: RaftTypeConfig,
    /// #     SM: RaftStateMachine<C>,
    /// # {
    /// // Admit the writes only while the leader that `as_leader()` returned still leads.
    /// let leader_id = raft.as_leader()?.to_committed_leader_id();
    /// raft.client_write_many([data1, data2], WriteOption::expect_leader(leader_id)).await?;
    /// # Ok(())
    /// # }
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

    /// Create an option that appends the write only if the leader's last log id is
    /// `last_log_id`.
    ///
    /// The same as `WriteOption::default().with_last_log_id(last_log_id)`; see
    /// [`with_last_log_id()`](Self::with_last_log_id).
    #[since(version = "0.10.2")]
    pub fn expect_last_log_id(last_log_id: LogIdOf<C>) -> Self {
        Self::default().with_last_log_id(last_log_id)
    }

    /// Append the write only if `last_log_id` is the leader's last log id right before the write.
    ///
    /// If it is not, the write fails with [`ClientWriteError::PreconditionFailed`] holding
    /// [`PreconditionFailed::LastLogIdMismatch`], which reports the actual last log id, and
    /// nothing is appended. A node that may not accept writes, such as a follower or a leader
    /// whose lease has expired, fails the write with [`ClientWriteError::ForwardToLeader`]
    /// before it compares the log, and so does an [expected leader](Self::with_leader) that does
    /// not match.
    ///
    /// The comparison is exact, so a stale `last_log_id`, such as a lagging one read from
    /// [`Raft::data_metrics()`], gets the write rejected. The rejection concerns this write only:
    /// whether another write is appended depends on the log id that the other write expects.
    ///
    /// To make a write depend on an earlier write, wait until the earlier write succeeds, then
    /// expect the log id returned for its last entry, as the example below does. The later write
    /// is then sent only after the earlier one is applied, and it is appended only if no other
    /// entry follows the earlier one.
    ///
    /// A write can also be sent before the write it depends on is answered, by expecting the log
    /// id that the earlier write's last entry will get. A leader gives its entries consecutive
    /// indexes, so the entry it appends after the one at `index` gets
    /// `LogId::new(leader.to_committed_leader_id(), index + 1)`, where `leader` is the [`Leader`]
    /// returned by [`Raft::as_leader()`].
    ///
    /// Such a prediction is safe only if the caller starts from that leader's actual last log id,
    /// and its dependent writes are the only entries appended to that leader's log from before it
    /// reads that log id until all of them are answered: no other write, blank entry or membership
    /// entry may be appended in that time. A log id names whichever entry the leader appended at
    /// that index, so otherwise a predicted log id may belong to another entry. The earlier write
    /// is then rejected, but a later write that expects that log id matches and is appended
    /// without the write it depends on. Metrics alone cannot establish the starting point: the
    /// last log id that [`Raft::data_metrics()`] publishes may lag entries the leader has already
    /// appended.
    ///
    /// RaftCore never merges such a write with other queued writes, so it compares the log right
    /// before the write's own entries.
    ///
    /// ```no_run
    /// # use openraft::Raft;
    /// # use openraft::RaftTypeConfig;
    /// # use openraft::storage::RaftStateMachine;
    /// use futures_util::TryStreamExt;
    /// use openraft::raft::WriteOption;
    ///
    /// # async fn example<C, SM>(
    /// #     raft: &Raft<C, SM>,
    /// #     data1: C::D,
    /// #     data2: C::D,
    /// # ) -> Result<(), Box<dyn std::error::Error>>
    /// # where
    /// #     C: RaftTypeConfig,
    /// #     SM: RaftStateMachine<C>,
    /// # {
    /// // The second write depends on the first, so send it only after the first succeeds.
    /// let first = raft.client_write(data1).await?;
    ///
    /// // Append the second write right after the first write's entry. If another entry follows
    /// // that entry, the second write fails with `PreconditionFailed`.
    /// let option = WriteOption::expect_last_log_id(first.log_id);
    /// let mut results = raft.client_write_many([data2], option).await?;
    /// let second = results.try_next().await?.expect("one result per write")?;
    /// println!("the second write is applied at {}", second.log_id);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`ClientWriteError::PreconditionFailed`]: crate::errors::ClientWriteError::PreconditionFailed
    /// [`PreconditionFailed::LastLogIdMismatch`]: crate::errors::PreconditionFailed::LastLogIdMismatch
    /// [`ClientWriteError::ForwardToLeader`]: crate::errors::ClientWriteError::ForwardToLeader
    /// [`Leader`]: crate::raft::Leader
    /// [`Raft::as_leader()`]: crate::Raft::as_leader
    /// [`Raft::data_metrics()`]: crate::Raft::data_metrics
    #[since(version = "0.10.2")]
    pub fn with_last_log_id(mut self, last_log_id: LogIdOf<C>) -> Self {
        self.expected_last_log_id = Some(last_log_id);
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
    ///
    /// An option with an expected last log id merges with no other option. The condition holds for
    /// the log right before the write: merged after another write, it would be compared before that
    /// write's entries are appended, and merged before one, it would also decide whether that write
    /// is appended.
    pub(crate) fn merge(&self, other: &Self) -> Option<Self> {
        if self.expected_leader != other.expected_leader {
            return None;
        }

        if self.expected_last_log_id.is_some() || other.expected_last_log_id.is_some() {
            return None;
        }

        Some(Self {
            expected_leader: self.expected_leader.clone(),
            expected_last_log_id: None,
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

    /// An option with an expected last log id merges with no option, not even an equal one.
    #[test]
    fn test_merge_expected_last_log_id() {
        let any = WriteOption::<C>::default();
        let last = WriteOption::<C>::default().with_last_log_id(log_id(1, 1, 3));
        let leader_and_last =
            WriteOption::<C>::expect_leader(committed_leader_id(1, 1)).with_last_log_id(log_id(1, 1, 3));
        let leader = WriteOption::<C>::expect_leader(committed_leader_id(1, 1));

        assert_eq!(None, any.merge(&last));
        assert_eq!(None, last.merge(&any));
        assert_eq!(None, last.merge(&last));

        assert_eq!(None, leader.merge(&leader_and_last));
        assert_eq!(None, leader_and_last.merge(&leader));
        assert_eq!(None, leader_and_last.merge(&leader_and_last));
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

    #[test]
    fn test_expect_last_log_id() {
        let last_log_id = log_id(1, 1, 3);

        assert_eq!(
            WriteOption::<C>::default().with_last_log_id(last_log_id),
            WriteOption::<C>::expect_last_log_id(last_log_id)
        );

        let option = WriteOption::<C>::expect_last_log_id(last_log_id);
        assert_eq!(Some(last_log_id), option.expected_last_log_id);
        assert_eq!(None, option.expected_leader);
    }
}
