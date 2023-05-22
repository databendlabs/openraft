use std::fmt;

use tokio::time::Instant;

use crate::leader::voting::Voting;
use crate::progress::entry::ProgressEntry;
use crate::progress::VecProgress;
use crate::quorum::QuorumSet;
use crate::utime::UTime;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::NodeId;
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
pub(crate) struct Leading<NID: NodeId, QS: QuorumSet<NID>> {
    // TODO(1): set the utime,
    // TODO(1): update it when heartbeat is granted by a quorum
    /// The vote this leader works in.
    pub(crate) vote: UTime<Vote<NID>>,

    /// Voting state, i.e., there is a Candidate running.
    voting: Voting<NID, QS>,

    /// Tracks the replication progress and committed index
    pub(crate) progress: VecProgress<NID, ProgressEntry<NID>, Option<LogId<NID>>, QS>,

    /// Tracks the clock time acknowledged by other nodes.
    ///
    /// See [`docs::leader_lease`] for more details.
    ///
    /// [`docs::leader_lease`]: `crate::docs::protocol::replication::leader_lease`
    pub(crate) clock_progress: VecProgress<NID, Option<Instant>, Option<Instant>, QS>,
}

impl<NID, QS> Leading<NID, QS>
where
    NID: NodeId,
    QS: QuorumSet<NID> + fmt::Debug + 'static,
{
    pub(crate) fn new(
        starting_voting_time: Instant,
        vote: Vote<NID>,
        quorum_set: QS,
        learner_ids: impl Iterator<Item = NID>,
        last_log_id: Option<LogId<NID>>,
    ) -> Self
    where
        QS: Clone,
    {
        let learner_ids = learner_ids.collect::<Vec<_>>();

        Self {
            vote: UTime::without_utime(vote),
            voting: Voting::new(starting_voting_time, vote, last_log_id, quorum_set.clone()),
            progress: VecProgress::new(
                quorum_set.clone(),
                learner_ids.iter().copied(),
                ProgressEntry::empty(last_log_id.next_index()),
            ),
            clock_progress: VecProgress::new(quorum_set, learner_ids, None),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn voting(&self) -> &Voting<NID, QS> {
        &self.voting
    }

    #[allow(dead_code)]
    pub(crate) fn voting_mut(&mut self) -> &mut Voting<NID, QS> {
        &mut self.voting
    }

    /// Update that a node has granted the vote.
    ///
    /// Return if a quorum has granted the vote.
    pub(crate) fn grant_vote_by(&mut self, target: NID) -> bool {
        self.voting.grant_by(&target)
    }
}
