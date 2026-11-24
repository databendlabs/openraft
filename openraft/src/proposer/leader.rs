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

/// The state of an established leader: a proposer whose vote is committed.
///
/// A node campaigns as a separate [`Candidate`], with an uncommitted vote. When an election quorum
/// grants that vote, [`Candidate::into_leader()`] commits it and builds a `Leader`, the state this
/// node uses to propose and replicate log entries.
///
/// The leader id of a `Leader` never changes. While this node leads, it starts no election unless
/// a leadership transfer has expired, and then the engine drops the `Leader` before this node
/// campaigns. A later leadership of this node, under a greater vote, gets a new `Leader`.
///
/// [`Candidate`]: crate::proposer::Candidate
/// [`Candidate::into_leader()`]: crate::proposer::Candidate::into_leader
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

    /// The vote that a quorum granted to this leader.
    ///
    /// Its leader id never changes while this `Leader` exists, and the log ids this leader assigns
    /// carry its committed leader id.
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

    /// Tracks RPC acknowledgments by sending stamp; acknowledgments from a write quorum keep this
    /// leader's lease.
    ///
    /// For each node, the value is the greatest [`SendStamp`] among the RPCs that the node
    /// acknowledged, or `None` before it acknowledges one. A stamp holds the sending time of the
    /// RPC, not its receiving time, and a heartbeat round that orders RPCs sent at the same time.
    /// Heartbeats, log replication requests, and snapshot transfers all count. A reply
    /// acknowledges an RPC when the node accepted this leader's vote, even if the reply reports a
    /// log conflict.
    ///
    /// This leader's own entry, present when the membership knows this node, starts at the time
    /// the `Leader` is built, with heartbeat round `0`. This leader grants each RPC it sends, but
    /// [`Self::update_clock()`] records that grant only when a node acknowledges the RPC, by
    /// raising this leader's entry to the RPC's stamp. [`Self::last_quorum_acked_stamp()`] and
    /// [`Self::last_quorum_acked_time()`] only read the stored stamps.
    ///
    /// A follower that accepts an RPC sent at `t` renews its lease on this leader's vote, and
    /// grants no vote to another candidate before `t + leader_lease`. This leader itself refuses
    /// votes while its lease is valid. Every [election quorum] is a read quorum, which shares a
    /// voter with every write quorum. So once every voter of a write quorum has acknowledged an RPC
    /// sent at `t` or later, no other candidate can win an election before `t + leader_lease`,
    /// except the target of a leadership transfer that this leader started.
    ///
    /// A leader that alone forms a write quorum keeps its lease without acknowledgments; see
    /// [`Self::is_lease_valid()`].
    ///
    /// See [`docs::leader_lease`] for more details.
    ///
    /// [election quorum]: crate::docs::protocol::election_quorum
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
    /// `last_log_id` is the last log id in the log; the leader proposes its noop log right after
    /// it. The vote must be greater than the leader id of `last_log_id`: a leader is never rebuilt
    /// with a leader id that its log already holds, because entries of that leader lost in a crash
    /// may survive on other nodes, and the rebuilt leader would assign their log ids to new
    /// entries.
    // leader_id: Copy is feature gated
    #[allow(clippy::clone_on_copy)]
    pub(crate) fn new(
        vote: CommittedVoteOf<C>,
        membership: C::Membership,
        last_log_id: Option<LogIdOf<C>>,
        id_gen: SharedIdGenerator,
    ) -> Self {
        let cl_id = vote.committed_leader_id();

        debug_assert!(
            Some(&cl_id) > last_log_id.as_ref().map(|x| x.committed_leader_id()),
            "vote {} must be greater than the leader id of last_log_id {:?}",
            vote,
            last_log_id
        );

        let non_write_voter_ids = membership.non_write_voter_ids().collect::<Vec<_>>();

        let noop_log_id = LogIdOf::<C>::new(cl_id, last_log_id.next_index());

        let write_quorum_set = Arc::new(membership.write_quorum_set());

        let progress = VecProgress::new(write_quorum_set.clone(), non_write_voter_ids.iter().cloned(), |id| {
            let stream_id = StreamId::new(id_gen.next_id());
            ProgressEntry::empty(id, stream_id, last_log_id.next_index()).with_probe_start(noop_log_id.index())
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
        let Some(deadline) = self.transfer_deadline() else {
            return false;
        };
        let now = C::now();
        now >= deadline
    }

    /// Return the instant by which the transfer target has to take over, if a transfer is in
    /// progress.
    pub(crate) fn transfer_deadline(&self) -> Option<InstantOf<C>> {
        let (_to, deadline) = self.transfer_to.as_ref()?;
        Some(*deadline)
    }

    /// Allocate a range of log IDs for new entries.
    ///
    /// Returns a [`LeaderLogIds`] containing the allocated log IDs, or `None` if count is 0.
    /// Updates `self.last_log_id` to the last allocated log ID.
    ///
    /// The caller is responsible for assigning the log IDs to entries.
    ///
    /// # Panics
    ///
    /// Panics in every build if the last allocated index would be greater than `u64::MAX - 1`, and
    /// leaves `self.last_log_id` unchanged. Iterating a [`LeaderLogIds`] needs the index after its
    /// last one, so a leader never allocates index `u64::MAX`.
    pub(crate) fn assign_log_ids(&mut self, count: usize) -> Option<LeaderLogIds<CommittedLeaderIdOf<C>>> {
        debug_assert!(self.transfer_to.is_none(), "leader is disabled to propose new log");

        if count == 0 {
            return None;
        }

        let committed_leader_id = self.committed_vote.committed_leader_id();
        let first = self.last_log_id().next_index();

        // Require the exclusive end, not only the last index, to fit in u64: iterating the range
        // needs it.
        let end = u64::try_from(count).ok().and_then(|n| first.checked_add(n));
        let Some(end) = end else {
            panic!("can not assign {count} log ids from index {first}: the last assignable log index is u64::MAX - 1");
        };
        let last = end - 1;

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
    ///
    /// A leader that alone forms a write quorum keeps its lease without acknowledgments, except
    /// while it transfers leadership. A transfer stops heartbeats so that the lease runs out and
    /// this leader votes for another candidate. Otherwise a transfer whose target never campaigns
    /// would keep this node leader until it steps down after the transfer deadline, refusing every
    /// write and every vote. A leader without `NodeRole::Campaigner` starts no election, so it
    /// would stay leader forever.
    pub(crate) fn is_lease_valid(&self, leader_lease: Duration) -> bool {
        let is_transferring = self.transfer_to.is_some();
        if self.is_self_quorum() && !is_transferring {
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
mod leader_test;
