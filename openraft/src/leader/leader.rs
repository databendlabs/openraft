use std::fmt;

use crate::leader::voting::Voting;
use crate::progress::entry::ProgressEntry;
use crate::progress::Progress;
use crate::progress::VecProgress;
use crate::quorum::QuorumSet;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
use crate::Instant;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::RaftLogId;
use crate::RaftTypeConfig;
use crate::Vote;

/// Leading state data.
///
/// Openraft leading state is the combination of Leader and Candidate in original raft.
/// A node becomes Leading at once when starting election, although at this time, it can not propose
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
pub(crate) struct Leading<C, QS: QuorumSet<C::NodeId>>
where C: RaftTypeConfig
{
    /// The vote this leader works in.
    ///
    /// `self.voting` may be in progress requesting vote for a higher vote.
    pub(crate) vote: Vote<C::NodeId>,

    last_log_id: Option<LogIdOf<C>>,

    /// The log id of the first log entry proposed by this leader,
    /// i.e., the `noop` log(AKA blank log) after leader established.
    ///
    /// It is set when leader established.
    pub(crate) noop_log_id: Option<LogIdOf<C>>,

    quorum_set: QS,

    /// Voting state, i.e., there is a Candidate running.
    voting: Option<Voting<C, QS>>,

    /// Tracks the replication progress and committed index
    pub(crate) progress: VecProgress<C::NodeId, ProgressEntry<C::NodeId>, Option<LogIdOf<C>>, QS>,

    /// Tracks the clock time acknowledged by other nodes.
    ///
    /// See [`docs::leader_lease`] for more details.
    ///
    /// [`docs::leader_lease`]: `crate::docs::protocol::replication::leader_lease`
    pub(crate) clock_progress: VecProgress<C::NodeId, Option<InstantOf<C>>, Option<InstantOf<C>>, QS>,
}

impl<C, QS> Leading<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId> + Clone + fmt::Debug + 'static,
{
    pub(crate) fn new(
        vote: Vote<C::NodeId>,
        quorum_set: QS,
        learner_ids: impl IntoIterator<Item = C::NodeId>,
        last_log_id: Option<LogIdOf<C>>,
    ) -> Self {
        let learner_ids = learner_ids.into_iter().collect::<Vec<_>>();

        Self {
            vote,
            last_log_id,
            noop_log_id: None,
            quorum_set: quorum_set.clone(),
            voting: None,
            progress: VecProgress::new(
                quorum_set.clone(),
                learner_ids.iter().copied(),
                ProgressEntry::empty(last_log_id.next_index()),
            ),
            clock_progress: VecProgress::new(quorum_set, learner_ids, None),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn voting(&self) -> Option<&Voting<C, QS>> {
        self.voting.as_ref()
    }

    #[allow(dead_code)]
    pub(crate) fn voting_mut(&mut self) -> Option<&mut Voting<C, QS>> {
        self.voting.as_mut()
    }

    /// Return the last log id this leader knows of.
    ///
    /// The leader's last log id may be different from the local RaftState.last_log_id.
    /// The later is used by the `Acceptor` part of a Raft node.
    pub(crate) fn last_log_id(&self) -> Option<&LogIdOf<C>> {
        self.last_log_id.as_ref()
    }

    /// Assign log ids to the entries.
    ///
    /// Return `()` if successful.
    /// Otherwise, return `Err(current_vote)` if this Leader is not yet established(by being
    /// accepted by a quorum).
    ///
    /// This method update the `self.last_log_id`.
    pub(crate) fn assign_log_ids<'a, LID: RaftLogId<C::NodeId> + 'a>(
        &mut self,
        entries: impl IntoIterator<Item = &'a mut LID>,
    ) -> Result<(), Vote<C::NodeId>> {
        let Some(committed_leader_id) = self.vote.committed_leader_id() else {
            return Err(self.vote);
        };

        let first = LogId::new(committed_leader_id, self.last_log_id().next_index());
        let mut last = first;

        for entry in entries {
            entry.set_log_id(&last);
            tracing::debug!("assign log id: {}", last);
            last.index += 1;
        }

        if last.index > first.index {
            last.index -= 1;
            self.last_log_id = Some(last);
        }

        Ok(())
    }

    /// Initialize a new voting process with specified vote, last_log_id and wall clock time.
    pub(crate) fn initialize_voting(
        &mut self,
        vote: Vote<C::NodeId>,
        last_log_id: Option<LogIdOf<C>>,
        now: InstantOf<C>,
    ) -> &mut Voting<C, QS> {
        self.voting = Some(Voting::new(now, vote, last_log_id, self.quorum_set.clone()));
        self.voting.as_mut().unwrap()
    }

    /// Finish the voting process and return the state.
    pub(crate) fn finish_voting(&mut self) -> Voting<C, QS> {
        // it has to be in voting progress
        self.voting.take().unwrap()
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
        let node_id = self.vote.leader_id().voted_for().unwrap();
        let now = Instant::now();

        tracing::debug!(
            leader_id = display(node_id),
            now = debug(now),
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
    use crate::engine::testing::UTConfig;
    use crate::entry::RaftEntry;
    use crate::leader::Leading;
    use crate::progress::Progress;
    use crate::testing::blank_ent;
    use crate::testing::log_id;
    use crate::type_config::alias::InstantOf;
    use crate::Entry;
    use crate::RaftLogId;
    use crate::Vote;

    #[test]
    fn test_leader_not_established() {
        let vote = Vote::new(2, 2);
        let mut leading = Leading::<UTConfig, _>::new(vote, vec![1, 2, 3], vec![], None);

        let mut entries = vec![Entry::<UTConfig>::new_blank(log_id(5, 5, 2))];
        let res = leading.assign_log_ids(&mut entries);

        assert_eq!(
            entries[0].get_log_id(),
            &log_id(5, 5, 2),
            "entry log id does not change"
        );
        assert_eq!(Err(Vote::new(2, 2)), res);
        assert_eq!(None, leading.last_log_id);
    }

    #[test]
    fn test_1_entry_none_last_log_id() {
        let vote = Vote::new(0, 0);
        let mut leading = Leading::<UTConfig, _>::new(vote, vec![1, 2, 3], vec![], None);

        let mut entries: Vec<Entry<UTConfig>> = vec![blank_ent(1, 1, 1)];
        let result = leading.assign_log_ids(&mut entries);

        assert!(result.is_ok());
        assert_eq!(entries[0].get_log_id(), &log_id(0, 0, 0),);
        assert_eq!(Some(log_id(0, 0, 0)), leading.last_log_id);
    }

    #[test]
    fn test_no_entries_provided() {
        let vote = Vote::new_committed(2, 2);
        let mut leading = Leading::<UTConfig, _>::new(vote, vec![1, 2, 3], vec![], Some(log_id(1, 1, 8)));

        let mut entries: Vec<Entry<UTConfig>> = vec![];
        let result = leading.assign_log_ids(&mut entries);
        assert!(result.is_ok());
        assert_eq!(Some(log_id(1, 1, 8)), leading.last_log_id);
    }

    #[test]
    fn test_multiple_entries() {
        let vote = Vote::new_committed(2, 2);
        let mut leading = Leading::<UTConfig, _>::new(vote, vec![1, 2, 3], [], Some(log_id(1, 1, 8)));

        let mut entries: Vec<Entry<UTConfig>> = vec![blank_ent(1, 1, 1), blank_ent(1, 1, 1), blank_ent(1, 1, 1)];

        let result = leading.assign_log_ids(&mut entries);
        assert!(result.is_ok());
        assert_eq!(entries[0].get_log_id(), &log_id(2, 2, 9));
        assert_eq!(entries[1].get_log_id(), &log_id(2, 2, 10));
        assert_eq!(entries[2].get_log_id(), &log_id(2, 2, 11));
        assert_eq!(Some(log_id(2, 2, 11)), leading.last_log_id);
    }

    #[test]
    fn test_leading_last_quorum_acked_time_leader_is_voter() {
        let mut leading = Leading::<UTConfig, Vec<u64>>::new(Vote::new_committed(2, 1), vec![1, 2, 3], [4], None);

        let now1 = InstantOf::<UTConfig>::now();

        let _t2 = leading.clock_progress.increase_to(&2, Some(now1));
        let t1 = leading.last_quorum_acked_time();
        assert_eq!(Some(now1), t1, "n1(leader) and n2 acked, t1 > t2");
    }

    #[test]
    fn test_leading_last_quorum_acked_time_leader_is_learner() {
        let mut leading = Leading::<UTConfig, Vec<u64>>::new(Vote::new_committed(2, 4), vec![1, 2, 3], [4], None);

        let t2 = InstantOf::<UTConfig>::now();
        let _ = leading.clock_progress.increase_to(&2, Some(t2));
        let t = leading.last_quorum_acked_time();
        assert!(t.is_none(), "n1(leader+learner) does not count in quorum");

        let t3 = InstantOf::<UTConfig>::now();
        let _ = leading.clock_progress.increase_to(&3, Some(t3));
        let t = leading.last_quorum_acked_time();
        assert_eq!(Some(t2), t, "n2 and n3 acked");
    }

    #[test]
    fn test_leading_last_quorum_acked_time_leader_is_not_member() {
        let mut leading = Leading::<UTConfig, Vec<u64>>::new(Vote::new_committed(2, 5), vec![1, 2, 3], [4], None);

        let t2 = InstantOf::<UTConfig>::now();
        let _ = leading.clock_progress.increase_to(&2, Some(t2));
        let t = leading.last_quorum_acked_time();
        assert!(t.is_none(), "n1(leader+learner) does not count in quorum");

        let t3 = InstantOf::<UTConfig>::now();
        let _ = leading.clock_progress.increase_to(&3, Some(t3));
        let t = leading.last_quorum_acked_time();
        assert_eq!(Some(t2), t, "n2 and n3 acked");
    }
}
