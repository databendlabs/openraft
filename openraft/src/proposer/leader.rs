use std::fmt;

use crate::LogIdOptionExt;
use crate::RaftTypeConfig;
use crate::display_ext::DisplayInstantExt;
use crate::engine::leader_log_ids::LeaderLogIds;
use crate::entry::RaftEntry;
use crate::entry::raft_entry_ext::RaftEntryExt;
use crate::progress::Progress;
use crate::progress::VecProgress;
use crate::progress::entry::ProgressEntry;
use crate::quorum::QuorumSet;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
use crate::vote::committed::CommittedVote;
use crate::vote::raft_vote::RaftVoteExt;

/// Leading state data.
///
/// Openraft leading state is the combination of Leader and Candidate in original raft.
/// A node becomes Leading at once when starting election, although at this time, it cannot propose
/// any new log, because its `vote` has not yet been granted by a quorum. I.e., A leader without
/// commit vote is a Candidate in original raft.
///
/// When the leader's vote is committed, i.e., granted by a quorum,
/// `Vote.committed` is set to true.
/// Then such a leader is the Leader in original raft.
///
/// By combining candidate and leader into one stage, openraft does not need to lose leadership when
/// a higher `leader_id`(roughly the `term` in original raft) is seen.
/// But instead it will be able to upgrade its `leader_id` without losing leadership.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct Leader<C, QS: QuorumSet<C::NodeId>>
where C: RaftTypeConfig
{
    /// Whether this Leader is marked as transferring to another node.
    ///
    /// Proposing is disabled when Leader has been transferring to another node.
    /// Indicates whether the current Leader is in the process of transferring leadership to another
    /// node.
    ///
    /// Leadership transfers disable proposing new logs.
    pub(crate) transfer_to: Option<C::NodeId>,

    /// The vote this leader works in.
    ///
    /// `self.voting` may be in progress requesting vote for a higher vote.
    pub(crate) committed_vote: CommittedVote<C>,

    /// The time to send next heartbeat.
    pub(crate) next_heartbeat: InstantOf<C>,

    last_log_id: Option<LogIdOf<C>>,

    /// The log id of the first log entry proposed by this leader,
    /// i.e., the `noop` log(AKA blank log) after leader established.
    ///
    /// It is set when leader established.
    pub(crate) noop_log_id: LogIdOf<C>,

    /// Tracks the replication progress and committed index
    pub(crate) progress: VecProgress<C::NodeId, ProgressEntry<C>, Option<LogIdOf<C>>, QS>,

    /// Tracks the clock time acknowledged by other nodes.
    ///
    /// Tracks the sending time(not receiving time) of the last heartbeat RPC to each follower.
    /// The leader's own entry is always updated with the current time when calculating
    /// the quorum-acknowledged time, as the leader is assumed to have the most up-to-date
    /// clock time. When a follower receives a heartbeat RPC, it resets its election timeout
    /// and won't start an election for at least the duration of `leader_lease`. If we denote
    /// the sending time of the heartbeat as `t`, then the leader can be sure that no follower
    /// can become a leader until `t + leader_lease`. This is the basis for the leader lease
    ///
    /// See [`docs::leader_lease`] for more details.
    ///
    /// [`docs::leader_lease`]: `crate::docs::protocol::replication::leader_lease`
    pub(crate) clock_progress: VecProgress<C::NodeId, Option<InstantOf<C>>, Option<InstantOf<C>>, QS>,
}

impl<C, QS> Leader<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId> + Clone + fmt::Debug + 'static,
{
    /// Create a new Leader.
    ///
    /// `last_leader_log_id` is the first and last log id proposed by the last leader.
    // leader_id: Copy is feature gated
    #[allow(clippy::clone_on_copy)]
    pub(crate) fn new(
        vote: CommittedVote<C>,
        quorum_set: QS,
        learner_ids: impl IntoIterator<Item = C::NodeId>,
        last_leader_log_id: LeaderLogIds<C>,
    ) -> Self {
        debug_assert!(
            Some(vote.committed_leader_id()) >= last_leader_log_id.last().map(|x| x.committed_leader_id().clone()),
            "vote {} must GE last_leader_log_id.last() {}",
            vote,
            last_leader_log_id
        );
        debug_assert!(
            Some(vote.committed_leader_id()) >= last_leader_log_id.first().map(|x| x.committed_leader_id().clone()),
            "vote {} must GE last_leader_log_id.first() {}",
            vote,
            last_leader_log_id
        );

        let learner_ids = learner_ids.into_iter().collect::<Vec<_>>();

        let vote_leader_id = vote.committed_leader_id();
        let first = last_leader_log_id.first();

        let noop_log_id = if first.map(|x| x.committed_leader_id()) == Some(&vote_leader_id) {
            // There is already log id proposed by this leader.
            // E.g. the Leader is restarted without losing leadership.
            //
            // Set to the first log id proposed by this Leader.
            //
            // Safe unwrap: first.map() == Some() is checked above.
            first.unwrap().clone()
        } else {
            // Set to a log id that will be proposed.
            LogIdOf::<C>::new(vote.committed_leader_id(), last_leader_log_id.last().next_index())
        };

        let last_log_id = last_leader_log_id.last().cloned();

        Self {
            transfer_to: None,
            committed_vote: vote,
            next_heartbeat: C::now(),
            last_log_id: last_log_id.clone(),
            noop_log_id,
            progress: VecProgress::new(quorum_set.clone(), learner_ids.iter().cloned(), || {
                ProgressEntry::empty(last_log_id.next_index())
            }),
            clock_progress: VecProgress::new(quorum_set, learner_ids, || None),
        }
    }

    pub(crate) fn noop_log_id(&self) -> &LogIdOf<C> {
        &self.noop_log_id
    }

    /// Return the last log id this leader knows of.
    ///
    /// The leader's last log id may be different from the local RaftState.last_log_id.
    /// The later is used by the `Acceptor` part of a Raft node.
    pub(crate) fn last_log_id(&self) -> Option<&LogIdOf<C>> {
        self.last_log_id.as_ref()
    }

    pub(crate) fn committed_vote_ref(&self) -> &CommittedVote<C> {
        &self.committed_vote
    }

    pub(crate) fn mark_transfer(&mut self, to: C::NodeId) {
        self.transfer_to = Some(to);
    }

    pub(crate) fn get_transfer_to(&self) -> Option<&C::NodeId> {
        self.transfer_to.as_ref()
    }

    /// Assign log ids to the entries.
    ///
    /// This method update the `self.last_log_id`.
    pub(crate) fn assign_log_ids<'a, Ent, I>(&mut self, entries: I)
    where
        Ent: RaftEntry<C> + 'a,
        I: IntoIterator<Item = &'a mut Ent>,
        <I as IntoIterator>::IntoIter: ExactSizeIterator,
    {
        debug_assert!(self.transfer_to.is_none(), "leader is disabled to propose new log");

        let it = entries.into_iter();
        let len = it.len();
        if len == 0 {
            return;
        }

        let committed_leader_id = self.committed_vote.committed_leader_id();

        let mut index = self.last_log_id().next_index();

        for entry in it {
            entry.set_log_id(LogIdOf::<C>::new(committed_leader_id.clone(), index));
            tracing::debug!("assign log id: {}", entry.ref_log_id());
            index += 1;
        }

        index -= 1;
        self.last_log_id = Some(LogIdOf::<C>::new(committed_leader_id.clone(), index));
    }

    /// Get the last timestamp acknowledged by a quorum.
    ///
    /// The acknowledgement by remote nodes are updated when AppendEntries reply is received.
    /// But if the time of the leader itself is not updated.
    ///
    /// Therefore everytime to retrieve the quorum acked timestamp, it should update with the
    /// leader's time first.
    /// It does not matter if the leader is not a voter, the QuorumSet will just ignore it.
    ///
    /// Note that the leader may not be in the QuorumSet at all.
    /// In such a case, the update operation will be just ignored,
    /// and the quorum-acked-time is totally determined by remove voters.
    pub(crate) fn last_quorum_acked_time(&mut self) -> Option<InstantOf<C>> {
        // For `Leading`, the vote is always the leader's vote.
        // Thus vote.voted_for() is this node.

        // Safe unwrap: voted_for() is always non-None in Openraft
        let node_id = self.committed_vote.to_leader_node_id().unwrap();
        let now = C::now();

        tracing::debug!(
            leader_id = display(&node_id),
            now = display(now.display()),
            "{}: update with leader's local time, before retrieving quorum acked clock",
            func_name!()
        );

        let granted = self.clock_progress.increase_to(&node_id, Some(now));

        match granted {
            Ok(x) => *x,
            // The leader node id may not be in the quorum set.
            Err(x) => *x,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Entry;
    use crate::Vote;
    use crate::engine::leader_log_ids::LeaderLogIds;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::entry::RaftEntry;
    use crate::progress::Progress;
    use crate::proposer::Leader;
    use crate::testing::blank_ent;
    use crate::type_config::TypeConfigExt;
    use crate::vote::raft_vote::RaftVoteExt;

    #[test]
    fn test_leader_new_with_proposed_log_id() {
        tracing::info!("--- vote greater than last log id, create new noop_log_id");
        {
            let vote = Vote::new(2, 2).into_committed();
            let leader = Leader::<UTConfig, _>::new(
                vote,
                vec![1, 2, 3],
                vec![],
                LeaderLogIds::new_start_end(log_id(1, 2, 1), log_id(1, 2, 3)),
            );

            assert_eq!(leader.noop_log_id(), &log_id(2, 2, 4));
            assert_eq!(leader.last_log_id(), Some(&log_id(1, 2, 3)));
        }

        tracing::info!("--- vote equals last log id, reuse noop_log_id");
        {
            let vote = Vote::new(1, 2).into_committed();
            let leader = Leader::<UTConfig, _>::new(
                vote,
                vec![1, 2, 3],
                vec![],
                LeaderLogIds::new_start_end(log_id(1, 2, 1), log_id(1, 2, 3)),
            );

            assert_eq!(leader.noop_log_id(), &log_id(1, 2, 1));
            assert_eq!(leader.last_log_id(), Some(&log_id(1, 2, 3)));
        }

        tracing::info!("--- vote equals last log id, reuse noop_log_id, last_leader_log_id.len()==1");
        {
            let vote = Vote::new(1, 2).into_committed();
            let leader =
                Leader::<UTConfig, _>::new(vote, vec![1, 2, 3], vec![], LeaderLogIds::new_single(log_id(1, 2, 3)));

            assert_eq!(leader.noop_log_id(), &log_id(1, 2, 3));
            assert_eq!(leader.last_log_id(), Some(&log_id(1, 2, 3)));
        }

        tracing::info!("--- no last log ids, create new noop_log_id, last_leader_log_id.len()==0");
        {
            let vote = Vote::new(1, 2).into_committed();
            let leader = Leader::<UTConfig, _>::new(vote, vec![1, 2, 3], vec![], LeaderLogIds::new(None));

            assert_eq!(leader.noop_log_id(), &log_id(1, 2, 0));
            assert_eq!(leader.last_log_id(), None);
        }
    }

    #[test]
    fn test_leader_established() {
        let vote = Vote::new(2, 2).into_committed();
        let mut leader =
            Leader::<UTConfig, _>::new(vote, vec![1, 2, 3], vec![], LeaderLogIds::new_single(log_id(1, 2, 3)));

        let mut entries = vec![Entry::<UTConfig>::new_blank(log_id(5, 5, 2))];
        leader.assign_log_ids(&mut entries);

        assert_eq!(
            entries[0].log_id(),
            log_id(2, 2, 4),
            "entry log id assigned following last-log-id"
        );
        assert_eq!(Some(log_id(2, 2, 4)), leader.last_log_id);
    }

    #[test]
    fn test_1_entry_none_last_log_id() {
        let vote = Vote::new(0, 0).into_committed();
        let mut leading = Leader::<UTConfig, _>::new(vote, vec![1, 2, 3], vec![], LeaderLogIds::new(None));

        let mut entries: Vec<Entry<UTConfig>> = vec![blank_ent(1, 1, 1)];
        leading.assign_log_ids(&mut entries);

        assert_eq!(entries[0].log_id(), log_id(0, 0, 0),);
        assert_eq!(Some(log_id(0, 0, 0)), leading.last_log_id);
    }

    #[test]
    fn test_no_entries_provided() {
        let vote = Vote::new(2, 2).into_committed();
        let mut leading =
            Leader::<UTConfig, _>::new(vote, vec![1, 2, 3], vec![], LeaderLogIds::new_single(log_id(1, 1, 8)));

        let mut entries: Vec<Entry<UTConfig>> = vec![];
        leading.assign_log_ids(&mut entries);
        assert_eq!(Some(log_id(1, 1, 8)), leading.last_log_id);
    }

    #[test]
    fn test_multiple_entries() {
        let vote = Vote::new(2, 2).into_committed();
        let mut leading =
            Leader::<UTConfig, _>::new(vote, vec![1, 2, 3], [], LeaderLogIds::new_single(log_id(1, 1, 8)));

        let mut entries: Vec<Entry<UTConfig>> = vec![blank_ent(1, 1, 1), blank_ent(1, 1, 1), blank_ent(1, 1, 1)];

        leading.assign_log_ids(&mut entries);
        assert_eq!(entries[0].log_id(), log_id(2, 2, 9));
        assert_eq!(entries[1].log_id(), log_id(2, 2, 10));
        assert_eq!(entries[2].log_id(), log_id(2, 2, 11));
        assert_eq!(Some(log_id(2, 2, 11)), leading.last_log_id);
    }

    #[test]
    fn test_leading_last_quorum_acked_time_leader_is_voter() {
        let mut leading = Leader::<UTConfig, Vec<u64>>::new(
            Vote::new(2, 1).into_committed(),
            vec![1, 2, 3],
            [4],
            LeaderLogIds::new(None),
        );

        let now1 = UTConfig::<()>::now();

        let _t2 = leading.clock_progress.increase_to(&2, Some(now1));
        let t1 = leading.last_quorum_acked_time();
        assert_eq!(Some(now1), t1, "n1(leader) and n2 acked, t1 > t2");
    }

    #[test]
    fn test_leading_last_quorum_acked_time_leader_is_learner() {
        let mut leading = Leader::<UTConfig, Vec<u64>>::new(
            Vote::new(2, 4).into_committed(),
            vec![1, 2, 3],
            [4],
            LeaderLogIds::new(None),
        );

        let t2 = UTConfig::<()>::now();
        let _ = leading.clock_progress.increase_to(&2, Some(t2));
        let t = leading.last_quorum_acked_time();
        assert!(t.is_none(), "n1(leader+learner) does not count in quorum");

        let t3 = UTConfig::<()>::now();
        let _ = leading.clock_progress.increase_to(&3, Some(t3));
        let t = leading.last_quorum_acked_time();
        assert_eq!(Some(t2), t, "n2 and n3 acked");
    }

    #[test]
    fn test_leading_last_quorum_acked_time_leader_is_not_member() {
        let mut leading = Leader::<UTConfig, Vec<u64>>::new(
            Vote::new(2, 5).into_committed(),
            vec![1, 2, 3],
            [4],
            LeaderLogIds::new(None),
        );

        let t2 = UTConfig::<()>::now();
        let _ = leading.clock_progress.increase_to(&2, Some(t2));
        let t = leading.last_quorum_acked_time();
        assert!(t.is_none(), "n1(leader+learner) does not count in quorum");

        let t3 = UTConfig::<()>::now();
        let _ = leading.clock_progress.increase_to(&3, Some(t3));
        let t = leading.last_quorum_acked_time();
        assert_eq!(Some(t2), t, "n2 and n3 acked");
    }
}
