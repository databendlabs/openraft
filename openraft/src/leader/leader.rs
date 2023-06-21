use std::fmt;
use std::ops::Deref;

use crate::leader::voting::Voting;
use crate::progress::entry::ProgressEntry;
use crate::progress::VecProgress;
use crate::quorum::QuorumSet;
use crate::utime::UTime;
use crate::Instant;
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
pub(crate) struct Leading<NID: NodeId, QS: QuorumSet<NID>, I: Instant> {
    // TODO(1): set the utime,
    // TODO(1): update it when heartbeat is granted by a quorum
    /// The vote this leader works in.
    pub(crate) vote: UTime<Vote<NID>, I>,

    quorum_set: QS,

    /// Voting state, i.e., there is a Candidate running.
    voting: Option<Voting<NID, QS, I>>,

    /// Tracks the replication progress and committed index
    pub(crate) progress: VecProgress<NID, ProgressEntry<NID>, Option<LogId<NID>>, QS>,

    /// Tracks the clock time acknowledged by other nodes.
    ///
    /// See [`docs::leader_lease`] for more details.
    ///
    /// [`docs::leader_lease`]: `crate::docs::protocol::replication::leader_lease`
    pub(crate) clock_progress: VecProgress<NID, Option<I>, Option<I>, QS>,
}

impl<NID, QS, I> Leading<NID, QS, I>
where
    NID: NodeId,
    QS: QuorumSet<NID> + Clone + fmt::Debug + 'static,
    I: Instant,
{
    pub(crate) fn new(
        vote: Vote<NID>,
        quorum_set: QS,
        learner_ids: impl Iterator<Item = NID>,
        last_log_id: Option<LogId<NID>>,
    ) -> Self {
        let learner_ids = learner_ids.collect::<Vec<_>>();

        Self {
            vote: UTime::without_utime(vote),
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
    pub(crate) fn voting(&self) -> Option<&Voting<NID, QS, I>> {
        self.voting.as_ref()
    }

    #[allow(dead_code)]
    pub(crate) fn voting_mut(&mut self) -> Option<&mut Voting<NID, QS, I>> {
        self.voting.as_mut()
    }

    pub(crate) fn initialize_voting(&mut self, last_log_id: Option<LogId<NID>>, now: I) -> &mut Voting<NID, QS, I> {
        self.voting = Some(Voting::new(
            now,
            *self.vote.deref(),
            last_log_id,
            self.quorum_set.clone(),
        ));
        self.voting.as_mut().unwrap()
    }

    /// Finish the voting process and return the state.
    pub(crate) fn finish_voting(&mut self) -> Voting<NID, QS, I> {
        // it has to be in voting progress
        self.voting.take().unwrap()
    }
}
