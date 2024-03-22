use std::fmt;

use crate::display_ext::DisplayOptionExt;
use crate::progress::Progress;
use crate::progress::VecProgress;
use crate::quorum::QuorumSet;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
use crate::RaftTypeConfig;
use crate::Vote;

/// Voting state.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct Voting<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId>,
{
    /// When the voting is started.
    starting_time: InstantOf<C>,

    /// The vote.
    vote: Vote<C::NodeId>,

    last_log_id: Option<LogIdOf<C>>,

    /// Which nodes have granted the the vote at certain time point.
    progress: VecProgress<C::NodeId, bool, bool, QS>,
}

impl<C, QS> fmt::Display for Voting<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId> + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{{}@{:?}, last_log_id:{} progress:{}}}",
            self.vote,
            self.starting_time,
            self.last_log_id.display(),
            self.progress
        )
    }
}

impl<C, QS> Voting<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId> + fmt::Debug + 'static,
{
    pub(crate) fn new(
        starting_time: InstantOf<C>,
        vote: Vote<C::NodeId>,
        last_log_id: Option<LogIdOf<C>>,
        quorum_set: QS,
    ) -> Self {
        Self {
            starting_time,
            vote,
            last_log_id,
            progress: VecProgress::new(quorum_set, [], false),
        }
    }

    pub(crate) fn vote_ref(&self) -> &Vote<C::NodeId> {
        &self.vote
    }

    pub(crate) fn progress(&self) -> &VecProgress<C::NodeId, bool, bool, QS> {
        &self.progress
    }

    /// Grant the vote by a node.
    pub(crate) fn grant_by(&mut self, target: &C::NodeId) -> bool {
        let granted = *self.progress.update(target, true).expect("target not in quorum set");

        tracing::info!(voting = debug(&self), "{}", func_name!());

        granted
    }

    /// Return the node ids that has granted this vote.
    #[allow(dead_code)]
    pub(crate) fn granters(&self) -> impl Iterator<Item = C::NodeId> + '_ {
        self.progress().iter().filter(|(_, granted)| *granted).map(|(target, _)| *target)
    }
}
