use std::fmt;

use crate::display_ext::DisplayInstantExt;
use crate::display_ext::DisplayOptionExt;
use crate::progress::Progress;
use crate::progress::VecProgress;
use crate::proposer::Leader;
use crate::quorum::QuorumSet;
use crate::type_config::alias::InstantOf;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::Vote;

/// Candidate: voting state.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct Candidate<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId>,
{
    /// When the voting is started.
    starting_time: InstantOf<C>,

    /// The vote.
    vote: Vote<C::NodeId>,

    last_log_id: Option<LogId<C::NodeId>>,

    /// Which nodes have granted the the vote at certain time point.
    progress: VecProgress<C::NodeId, bool, bool, QS>,

    quorum_set: QS,

    learner_ids: Vec<C::NodeId>,
}

impl<C, QS> fmt::Display for Candidate<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId> + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{{}@{}, last_log_id:{} progress:{}}}",
            self.vote,
            self.starting_time.display(),
            self.last_log_id.display(),
            self.progress
        )
    }
}

impl<C, QS> Candidate<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId> + fmt::Debug + Clone + 'static,
{
    pub(crate) fn new(
        starting_time: InstantOf<C>,
        vote: Vote<C::NodeId>,
        last_log_id: Option<LogId<C::NodeId>>,
        quorum_set: QS,
        learner_ids: impl IntoIterator<Item = C::NodeId>,
    ) -> Self {
        Self {
            starting_time,
            vote,
            last_log_id,
            progress: VecProgress::new(quorum_set.clone(), [], false),
            quorum_set,
            learner_ids: learner_ids.into_iter().collect::<Vec<_>>(),
        }
    }

    pub(crate) fn vote_ref(&self) -> &Vote<C::NodeId> {
        &self.vote
    }

    pub(crate) fn last_log_id(&self) -> Option<&LogId<C::NodeId>> {
        self.last_log_id.as_ref()
    }

    pub(crate) fn progress(&self) -> &VecProgress<C::NodeId, bool, bool, QS> {
        &self.progress
    }

    /// Grant the vote by a node.
    pub(crate) fn grant_by(&mut self, target: &C::NodeId) -> bool {
        let granted = *self.progress.update(target, true).expect("target not in quorum set");

        tracing::info!(voting = display(&self), "{}", func_name!());

        granted
    }

    /// Return the node ids that has granted this vote.
    #[allow(dead_code)]
    pub(crate) fn granters(&self) -> impl Iterator<Item = C::NodeId> + '_ {
        self.progress().iter().filter(|(_, granted)| *granted).map(|(target, _)| target.clone())
    }

    pub(crate) fn into_leader(self) -> Leader<C, QS> {
        // Mark the vote as committed, i.e., being granted and saved by a quorum.
        let vote = {
            let mut vote = self.vote_ref().clone();
            debug_assert!(!vote.is_committed());
            vote.commit();
            vote
        };

        let last_leader_log_ids = self.last_log_id().cloned().into_iter().collect::<Vec<_>>();

        Leader::new(vote, self.quorum_set.clone(), self.learner_ids, &last_leader_log_ids)
    }
}
