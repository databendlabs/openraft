use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use validit::Valid;

use crate::LogIdOptionExt;
use crate::RaftTypeConfig;
use crate::base::shared_id_generator::SharedIdGenerator;
use crate::engine::leader_log_ids::LeaderLogIds;
use crate::errors::QuorumNotEnough;
use crate::membership::RaftMembership;
use crate::membership::RaftMembershipExt;
use crate::progress::IdVal;
use crate::progress::VecProgress;
use crate::progress::entry::ProgressEntry;
use crate::progress::stream_id::StreamId;
use crate::proposer::SendStamp;
use crate::proposer::WriteQuorumSet;
use crate::quorum::QuorumSet;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::CommittedVoteOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
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
pub(crate) struct Leader<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<Id = C::NodeId>,
{
    /// The node this Leader is transferring leadership to, and the deadline of the transfer.
    ///
    /// Proposing is disabled when Leader has been transferring to another node.
    /// Indicates whether the current Leader is in the process of transferring leadership to another
    /// node.
    ///
    /// Leadership transfers disable proposing new logs.
    ///
    /// If no other node has taken over by the deadline, this Leader steps down and starts an
    /// election, so that a failed transfer does not leave the cluster without a serving leader.
    pub(crate) transfer_to: Option<(C::NodeId, InstantOf<C>)>,

    /// The vote this leader works in.
    ///
    /// `self.voting` may be in progress requesting vote for a higher vote.
    pub(crate) committed_vote: CommittedVoteOf<C>,

    /// The time to send next heartbeat.
    pub(crate) next_heartbeat: InstantOf<C>,

    /// The round of the last heartbeat broadcast, or `0` if none has been broadcast.
    ///
    /// A read records this round when it arrives; only a heartbeat broadcast after that has a
    /// higher round. See [`SendStamp`].
    heartbeat_round: u64,

    last_log_id: Option<LogIdOf<C>>,

    /// The log id of the first log entry proposed by this leader,
    /// i.e., the `noop` log(AKA blank log) after leader established.
    ///
    /// It is set when leader established.
    pub(crate) noop_log_id: LogIdOf<C>,

    /// Tracks replication to all known nodes; only write voters determine the committed index.
    pub(crate) progress: Valid<VecProgress<ProgressEntry<C>, QS>>,

    /// Tracks heartbeat acknowledgments; a write quorum confirms the leader's authority.
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
    /// Each value is a [`SendStamp`]: the sending time plus a heartbeat round that orders RPCs sent
    /// at the same time.
    ///
    /// [`docs::leader_lease`]: `crate::docs::protocol::replication::leader_lease`
    pub(crate) clock_progress: Valid<VecProgress<IdVal<C::NodeId, Option<SendStamp<InstantOf<C>>>>, QS>>,

    /// Whether this leader alone forms a write quorum; see [`Self::is_self_quorum()`].
    ///
    /// [`QuorumSet::is_quorum()`] runs application code with no bound on its running time. So
    /// [`Self::update_self_quorum()`] computes this value once per write quorum set, instead of on
    /// every lease check, read, and metrics report.
    self_quorum: bool,
}

impl<C> Leader<C, WriteQuorumSet<C>>
where C: RaftTypeConfig
{
    /// Creates a leader using the membership's write quorum for replication and heartbeat tracking.
    ///
    /// `last_leader_log_ids` is the first and last log id proposed by the last leader.
    // leader_id: Copy is feature gated
    #[allow(clippy::clone_on_copy)]
    pub(crate) fn new(
        vote: CommittedVoteOf<C>,
        membership: C::Membership,
        last_leader_log_ids: Option<LeaderLogIds<CommittedLeaderIdOf<C>>>,
        id_gen: SharedIdGenerator,
    ) -> Self {
        let cl_id = vote.committed_leader_id();

        if let Some(ref log_ids) = last_leader_log_ids {
            debug_assert!(
                Some(&cl_id) >= Some(log_ids.last_ref().committed_leader_id()),
                "vote {} must GE last_leader_log_ids.last_log_id() {:?}",
                vote,
                last_leader_log_ids
            );
            debug_assert!(
                Some(&cl_id) >= Some(log_ids.first_ref().committed_leader_id()),
                "vote {} must GE last_leader_log_ids.first_log_id() {:?}",
                vote,
                last_leader_log_ids
            );
        }

        let non_write_voter_ids = membership.non_write_voter_ids().collect::<Vec<_>>();

        let first_ref = last_leader_log_ids.as_ref().map(|x| x.first_ref());
        let last_ref = last_leader_log_ids.as_ref().map(|x| x.last_ref());

        let noop_log_id = if first_ref.as_ref().map(|x| x.committed_leader_id()) == Some(&cl_id) {
            // There is already log id proposed by this leader.
            // E.g. the Leader is restarted without losing leadership.
            //
            // Set to the first log id proposed by this Leader.
            //
            // Safe unwrap: first.map() == Some() is checked above.
            first_ref.unwrap().into_log_id()
        } else {
            // Set to a log id that will be proposed.
            LogIdOf::<C>::new(cl_id, last_ref.next_index())
        };

        let last_log_id = last_ref.map(|r| r.into_log_id());

        let write_quorum_set = Arc::new(membership.write_quorum_set().clone());

        let progress = VecProgress::new(write_quorum_set.clone(), non_write_voter_ids.iter().cloned(), |id| {
            let stream_id = StreamId::new(id_gen.next_id());
            ProgressEntry::empty(id, stream_id, last_log_id.next_index())
        });

        let now = C::now();
        let mut clock_progress = VecProgress::new(write_quorum_set, non_write_voter_ids, IdVal::new_default);
        let leader_node_id = vote.to_leader_node_id();
        clock_progress.increase_to(&leader_node_id, Some(SendStamp::new(now, 0)));

        let mut leader = Self {
            transfer_to: None,
            committed_vote: vote,
            next_heartbeat: now,
            heartbeat_round: 0,
            last_log_id: last_log_id.clone(),
            noop_log_id,
            progress: Valid::new(progress),
            clock_progress: Valid::new(clock_progress),
            self_quorum: false,
        };
        leader.update_self_quorum();
        leader
    }
}

impl<C, QS> Leader<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<Id = C::NodeId> + Clone + fmt::Debug + 'static,
{
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

    pub(crate) fn committed_vote_ref(&self) -> &CommittedVoteOf<C> {
        &self.committed_vote
    }

    /// Mark this Leader as transferring leadership to `to`, which has to take over within
    /// `timeout`.
    pub(crate) fn mark_transfer(&mut self, to: C::NodeId, timeout: Duration) {
        let deadline = C::now() + timeout;
        self.transfer_to = Some((to, deadline));
    }

    pub(crate) fn get_transfer_to(&self) -> Option<&C::NodeId> {
        let (to, _deadline) = self.transfer_to.as_ref()?;
        Some(to)
    }

    /// Return whether a leadership transfer is in progress and its deadline has passed.
    ///
    /// Such a Leader steps down and starts an election.
    pub(crate) fn is_transfer_expired(&self) -> bool {
        let Some((_to, deadline)) = self.transfer_to.as_ref() else {
            return false;
        };
        let now = C::now();
        now >= *deadline
    }

    /// Allocate a range of log IDs for new entries.
    ///
    /// Returns a [`LeaderLogIds`] containing the allocated log IDs, or `None` if count is 0.
    /// Updates `self.last_log_id` to the last allocated log ID.
    ///
    /// The caller is responsible for assigning the log IDs to entries.
    pub(crate) fn assign_log_ids(&mut self, count: usize) -> Option<LeaderLogIds<CommittedLeaderIdOf<C>>> {
        debug_assert!(self.transfer_to.is_none(), "leader is disabled to propose new log");

        if count == 0 {
            return None;
        }

        let committed_leader_id = self.committed_vote.committed_leader_id();
        let first = self.last_log_id().next_index();
        let last = first + count as u64 - 1;

        self.last_log_id = Some(LogIdOf::<C>::new(committed_leader_id.clone(), last));

        Some(LeaderLogIds::new(committed_leader_id, first, last))
    }

    /// Start a new heartbeat round and return it.
    pub(crate) fn next_heartbeat_round(&mut self) -> u64 {
        let round = self.heartbeat_round.checked_add(1).expect("heartbeat round overflow");
        self.heartbeat_round = round;
        round
    }

    /// Return the round of the last heartbeat broadcast, or `0` if none has been broadcast.
    pub(crate) fn heartbeat_round(&self) -> u64 {
        self.heartbeat_round
    }

    /// Update the clock acknowledged by `target` and return the stamp acknowledged by a quorum.
    pub(crate) fn update_clock(
        &mut self,
        target: &C::NodeId,
        sending_time: SendStamp<InstantOf<C>>,
    ) -> Option<SendStamp<InstantOf<C>>> {
        let leader_node_id = self.committed_vote.to_leader_node_id();
        self.clock_progress.increase_to(&leader_node_id, Some(sending_time));

        *self
            .clock_progress
            .increase_to(target, Some(sending_time))
            .expect("the target must exist in clock progress")
    }

    /// Get the last timestamp acknowledged by a quorum.
    pub(crate) fn last_quorum_acked_time(&self) -> Option<InstantOf<C>> {
        let acked = self.last_quorum_acked_stamp()?;
        Some(acked.time)
    }

    /// Get the last [`SendStamp`] acknowledged by a quorum.
    pub(crate) fn last_quorum_acked_stamp(&self) -> Option<SendStamp<InstantOf<C>>> {
        *self.clock_progress.quorum_accepted()
    }

    /// Report the clock progress as a [`QuorumNotEnough`] error.
    ///
    /// The reported `got` set names the voters whose last acknowledged RPC was sent after
    /// `min_acked_at`, so the caller can see how far the clock is from a quorum. Learners are left
    /// out because they never grant a value. This leader is always included when it is a voter: it
    /// grants its own RPCs, while [`Self::update_clock`] records that grant only once a follower
    /// responds.
    pub(crate) fn clock_quorum_not_enough(&self, min_acked_at: SendStamp<InstantOf<C>>) -> QuorumNotEnough<C> {
        let voter_count = self.clock_progress.voter_count();

        let mut got: BTreeSet<C::NodeId> = self
            .clock_progress
            .iter()
            .take(voter_count)
            .filter(|entry| entry.val.is_some_and(|acked_at| acked_at > min_acked_at))
            .map(|entry| entry.id.clone())
            .collect();

        let leader_node_id = self.committed_vote.to_leader_node_id();
        if self.clock_progress.is_voter(&leader_node_id) == Some(true) {
            got.insert(leader_node_id);
        }

        QuorumNotEnough {
            cluster: format!("{:?}", self.clock_progress.quorum_set()),
            got,
        }
    }

    /// Return whether this leader's lease is valid.
    pub(crate) fn is_lease_valid(&self, leader_lease: Duration) -> bool {
        if self.is_self_quorum() {
            return true;
        }

        let now = C::now();
        self.last_quorum_acked_time().is_some_and(|acked| now < acked + leader_lease)
    }

    /// Return whether this leader alone forms a write quorum.
    ///
    /// Such a leader needs no acknowledgment to keep its lease. Every read quorum must share a node
    /// with the write quorum `{leader}`, so every read quorum contains this leader, and no
    /// candidate can win without its vote. The leader refuses that vote while it leads, except
    /// for a leadership transfer that it started.
    pub(crate) fn is_self_quorum(&self) -> bool {
        #[cfg(debug_assertions)]
        {
            let calculated = self.calc_self_quorum();
            debug_assert_eq!(
                calculated, self.self_quorum,
                "update_self_quorum() must run after clock_progress gets a new quorum set"
            );
        }

        self.self_quorum
    }

    /// Recompute [`Self::self_quorum`]. Call it after [`Self::clock_progress`] gets a new quorum
    /// set.
    pub(crate) fn update_self_quorum(&mut self) {
        self.self_quorum = self.calc_self_quorum();
    }

    fn calc_self_quorum(&self) -> bool {
        let leader_node_id = self.committed_vote.to_leader_node_id();
        let write_quorum_set = self.clock_progress.quorum_set();
        write_quorum_set.is_quorum(std::iter::once(&leader_node_id))
    }

    /// Decide whether a heartbeat needs to be sent to `target` at time `now`.
    ///
    /// A heartbeat is redundant if `target` has recently acknowledged an RPC (log replication
    /// or heartbeat): the acknowledgment proves the follower's liveness and already extended
    /// the leader lease via [`Self::clock_progress`], which records the *sending* time of the
    /// last acknowledged RPC.
    ///
    /// `min_interval` is [`Config::heartbeat_min_interval`]; `0` disables suppression so that
    /// a heartbeat is always sent.
    ///
    /// [`Config::heartbeat_min_interval`]: `crate::Config::heartbeat_min_interval`
    pub(crate) fn need_heartbeat(&self, target: &C::NodeId, now: InstantOf<C>, min_interval: Duration) -> bool {
        let acked = self.clock_progress.try_get(target).and_then(|entry| entry.val);
        acked.is_none_or(|sending_time| now >= sending_time.time + min_interval)
    }

    pub(crate) fn is_replication_stream_valid(&self, target: &C::NodeId, stream_id: StreamId) -> bool {
        if let Some(entry) = self.progress.try_get(target)
            && entry.data.stream_id == stream_id
        {
            return true;
        }

        tracing::warn!(
            "{}: target node {} stream_id:{} not found in progress tracker. It may be from a delayed message, ignore",
            func_name!(),
            target,
            stream_id,
        );

        false
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use maplit::btreeset;

    use crate::Membership;
    use crate::Vote;
    use crate::base::shared_id_generator::SharedIdGenerator;
    use crate::engine::leader_log_ids::LeaderLogIds;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::proposer::Leader;
    use crate::proposer::SendStamp;
    use crate::type_config::TypeConfigExt;
    use crate::vote::raft_vote::RaftVoteExt;

    #[test]
    fn test_new_tracks_write_voters_and_other_nodes() {
        let membership = Membership::new_with_defaults(vec![btreeset! {1,2}, btreeset! {2,3}], [4, 5]);
        let vote = Vote::new(2, 1).to_committed();
        let leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

        let progress_ids = leading.progress.iter().map(|entry| entry.id);
        let progress_ids = progress_ids.collect::<BTreeSet<_>>();
        assert_eq!(btreeset! {1,2,3,4,5}, progress_ids);
        let clock_ids = leading.clock_progress.iter().map(|entry| entry.id);
        let clock_ids = clock_ids.collect::<BTreeSet<_>>();
        assert_eq!(btreeset! {1,2,3,4,5}, clock_ids);
        let write_voters = (0..=6).filter(|id| leading.progress.is_voter(id) == Some(true)).collect::<Vec<_>>();
        assert_eq!(vec![1, 2, 3], write_voters);
        let clock_voters = (0..=6).filter(|id| leading.clock_progress.is_voter(id) == Some(true)).collect::<Vec<_>>();
        assert_eq!(vec![1, 2, 3], clock_voters);
    }

    #[test]
    fn test_joint_clock_quorum_excludes_non_write_voters() {
        let membership = Membership::new_with_defaults(vec![btreeset! {1,2}, btreeset! {2,3}], [4]);
        let vote = Vote::new(2, 1).to_committed();
        let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());
        let now = UTConfig::<()>::now();
        let stamp = SendStamp::new(now, 0);

        let acked = leading.update_clock(&2, stamp);
        assert_eq!(None, acked);
        let acked = leading.update_clock(&4, stamp);
        assert_eq!(None, acked);
        let acked = leading.update_clock(&3, stamp);
        assert_eq!(Some(stamp), acked);
    }

    #[test]
    fn test_leader_new_with_proposed_log_id() {
        tracing::info!("--- vote greater than last log id, create new noop_log_id");
        {
            let vote = Vote::new(2, 2).to_committed();
            let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
            let leader = Leader::<UTConfig, _>::new(
                vote,
                membership,
                Some(LeaderLogIds::new(*log_id(1, 2, 0).committed_leader_id(), 1, 3)),
                SharedIdGenerator::new(),
            );

            assert_eq!(leader.noop_log_id(), &log_id(2, 2, 4));
            assert_eq!(leader.last_log_id(), Some(&log_id(1, 2, 3)));
        }

        tracing::info!("--- vote equals last log id, reuse noop_log_id");
        {
            let vote = Vote::new(1, 2).to_committed();
            let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
            let leader = Leader::<UTConfig, _>::new(
                vote,
                membership,
                Some(LeaderLogIds::new(*log_id(1, 2, 0).committed_leader_id(), 1, 3)),
                SharedIdGenerator::new(),
            );

            assert_eq!(leader.noop_log_id(), &log_id(1, 2, 1));
            assert_eq!(leader.last_log_id(), Some(&log_id(1, 2, 3)));
        }

        tracing::info!("--- vote equals last log id, reuse noop_log_id, last_leader_log_id.len()==1");
        {
            let vote = Vote::new(1, 2).to_committed();
            let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
            let leader = Leader::<UTConfig, _>::new(
                vote,
                membership,
                Some(LeaderLogIds::new_single(log_id(1, 2, 3))),
                SharedIdGenerator::new(),
            );

            assert_eq!(leader.noop_log_id(), &log_id(1, 2, 3));
            assert_eq!(leader.last_log_id(), Some(&log_id(1, 2, 3)));
        }

        tracing::info!("--- no last log ids, create new noop_log_id, last_leader_log_id.len()==0");
        {
            let vote = Vote::new(1, 2).to_committed();
            let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
            let leader = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

            assert_eq!(leader.noop_log_id(), &log_id(1, 2, 0));
            assert_eq!(leader.last_log_id(), None);
        }
    }

    #[test]
    fn test_leader_established() {
        let vote = Vote::new(2, 2).to_committed();
        let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
        let mut leader = Leader::<UTConfig, _>::new(
            vote,
            membership,
            Some(LeaderLogIds::new_single(log_id(1, 2, 3))),
            SharedIdGenerator::new(),
        );

        let log_ids: Vec<_> = leader.assign_log_ids(1).unwrap().into_iter().collect();

        assert_eq!(
            log_ids,
            vec![log_id(2, 2, 4)],
            "entry log id assigned following last-log-id"
        );
        assert_eq!(Some(log_id(2, 2, 4)), leader.last_log_id);
    }

    #[test]
    fn test_1_entry_none_last_log_id() {
        let vote = Vote::new(0, 0).to_committed();
        let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
        let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

        let log_ids: Vec<_> = leading.assign_log_ids(1).unwrap().into_iter().collect();

        assert_eq!(log_ids, vec![log_id(0, 0, 0)]);
        assert_eq!(Some(log_id(0, 0, 0)), leading.last_log_id);
    }

    #[test]
    fn test_no_entries_provided() {
        let vote = Vote::new(2, 2).to_committed();
        let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
        let mut leading = Leader::<UTConfig, _>::new(
            vote,
            membership,
            Some(LeaderLogIds::new_single(log_id(1, 1, 8))),
            SharedIdGenerator::new(),
        );

        let log_ids = leading.assign_log_ids(0);
        assert_eq!(log_ids, None);
        assert_eq!(Some(log_id(1, 1, 8)), leading.last_log_id);
    }

    #[test]
    fn test_multiple_entries() {
        let vote = Vote::new(2, 2).to_committed();
        let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
        let mut leading = Leader::<UTConfig, _>::new(
            vote,
            membership,
            Some(LeaderLogIds::new_single(log_id(1, 1, 8))),
            SharedIdGenerator::new(),
        );

        let log_ids: Vec<_> = leading.assign_log_ids(3).unwrap().into_iter().collect();
        assert_eq!(log_ids, vec![log_id(2, 2, 9), log_id(2, 2, 10), log_id(2, 2, 11)]);
        assert_eq!(Some(log_id(2, 2, 11)), leading.last_log_id);
    }

    #[test]
    fn test_leading_last_quorum_acked_time_leader_is_voter() {
        let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
        let vote = Vote::new(2, 1).to_committed();
        let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

        let now1 = UTConfig::<()>::now();

        leading.update_clock(&2, SendStamp::new(now1, 0));
        let t1 = leading.last_quorum_acked_time();
        assert_eq!(Some(now1), t1, "n1(leader) and n2 acked, t1 > t2");
    }

    #[test]
    fn test_leading_last_quorum_acked_time_leader_is_learner() {
        let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
        let vote = Vote::new(2, 4).to_committed();
        let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

        let t2 = UTConfig::<()>::now();
        leading.update_clock(&2, SendStamp::new(t2, 0));
        let t = leading.last_quorum_acked_time();
        assert!(t.is_none(), "n1(leader+learner) does not count in quorum");

        let t3 = UTConfig::<()>::now();
        leading.update_clock(&3, SendStamp::new(t3, 0));
        let t = leading.last_quorum_acked_time();
        assert_eq!(Some(t2), t, "n2 and n3 acked");
    }

    #[test]
    fn test_leading_last_quorum_acked_time_leader_is_not_member() {
        let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
        let vote = Vote::new(2, 5).to_committed();
        let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

        let t2 = UTConfig::<()>::now();
        leading.update_clock(&2, SendStamp::new(t2, 0));
        let t = leading.last_quorum_acked_time();
        assert!(t.is_none(), "n1(leader+learner) does not count in quorum");

        let t3 = UTConfig::<()>::now();
        leading.update_clock(&3, SendStamp::new(t3, 0));
        let t = leading.last_quorum_acked_time();
        assert_eq!(Some(t2), t, "n2 and n3 acked");
    }

    #[test]
    fn test_clock_quorum_not_enough() {
        // Voters {1,2,3} with learner {4}; node 1 is the leader.
        let membership = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
        let cluster = "[{1, 2, 3}]";
        let vote = Vote::new(2, 1).to_committed();
        let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

        let now = UTConfig::<()>::now();
        let t1 = SendStamp::new(now, 0);
        let t2 = SendStamp::new(now + Duration::from_millis(10), 0);

        let err = leading.clock_quorum_not_enough(t1);
        assert_eq!(
            btreeset! {1},
            err.got,
            "the leader grants its own RPCs before any response"
        );
        assert_eq!(cluster, err.cluster);

        leading.update_clock(&2, t1);
        let err = leading.clock_quorum_not_enough(t1);
        assert_eq!(btreeset! {1}, err.got, "an RPC sent at t1 is not newer than t1");

        leading.update_clock(&2, t2);
        let err = leading.clock_quorum_not_enough(t1);
        assert_eq!(btreeset! {1, 2}, err.got, "n2 acked an RPC sent after t1");

        leading.update_clock(&4, t2);
        let err = leading.clock_quorum_not_enough(t2);
        assert_eq!(
            btreeset! {1},
            err.got,
            "learner n4 is not counted, and n2's ack is too old"
        );
    }

    /// The clock may not move between a read and the heartbeat it triggers, as on a simulated
    /// runtime. The heartbeat round still tells an RPC sent before the read from one sent after it.
    #[test]
    fn test_clock_quorum_orders_same_time_by_heartbeat_round() {
        // Voters {1,2,3} with learner {4}; node 1 is the leader.
        let membership = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
        let vote = Vote::new(2, 1).to_committed();
        let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());
        let now = UTConfig::<()>::now();

        tracing::info!("--- n2 acks a replication RPC sent at `now`, before the read arrives");
        leading.update_clock(&2, SendStamp::new(now, 0));

        let round_at_read = leading.heartbeat_round();
        let read_threshold = SendStamp::new(now, round_at_read);

        tracing::info!(round_at_read, "--- an RPC sent before the read does not confirm it");
        {
            let err = leading.clock_quorum_not_enough(read_threshold);
            assert_eq!(btreeset! {1}, err.got);
        }

        tracing::info!("--- n3 acks the heartbeat that the read triggers, still at `now`");
        {
            let round = leading.next_heartbeat_round();
            let heartbeat = SendStamp::new(now, round);
            leading.update_clock(&3, heartbeat);

            let err = leading.clock_quorum_not_enough(read_threshold);
            assert_eq!(btreeset! {1, 3}, err.got);

            let quorum_acked = leading.last_quorum_acked_stamp();
            assert_eq!(Some(heartbeat), quorum_acked);
        }
    }

    #[test]
    fn test_need_heartbeat() {
        let membership = Membership::new_with_defaults(vec![btreeset! {1, 2, 3}], [4]);
        let vote = Vote::new(2, 1).to_committed();
        let mut leading = Leader::<UTConfig, _>::new(vote, membership, None, SharedIdGenerator::new());

        let min_interval = Duration::from_millis(100);
        let t0 = UTConfig::<()>::now();

        assert!(
            leading.need_heartbeat(&2, t0, min_interval),
            "never acknowledged: must send"
        );
        assert!(
            leading.need_heartbeat(&9, t0, min_interval),
            "unknown target: must send"
        );

        leading.clock_progress.increase_to(&2, Some(SendStamp::new(t0, 0)));

        assert!(
            !leading.need_heartbeat(&2, t0 + Duration::from_millis(99), min_interval),
            "acked RPC sent within min_interval: suppressed"
        );
        assert!(
            leading.need_heartbeat(&2, t0 + Duration::from_millis(100), min_interval),
            "acked RPC sending time is min_interval old: must send"
        );
        assert!(
            leading.need_heartbeat(&2, t0, Duration::ZERO),
            "zero min_interval disables suppression"
        );
        assert!(
            leading.need_heartbeat(&3, t0 + Duration::from_millis(99), min_interval),
            "only the acked follower is suppressed"
        );
    }
}
