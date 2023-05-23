use std::fmt;

use tokio::time::Instant;

use crate::display_ext::DisplayOptionExt;
use crate::progress::Progress;
use crate::progress::VecProgress;
use crate::quorum::QuorumSet;
use crate::LogId;
use crate::NodeId;
use crate::Vote;

/// Voting state.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct Voting<NID, QS>
where
    NID: NodeId,
    QS: QuorumSet<NID>,
{
    /// When the voting is started.
    starting_time: Instant,

    /// The vote.
    vote: Vote<NID>,

    last_log_id: Option<LogId<NID>>,

    /// Which nodes have granted the the vote at certain time point.
    progress: VecProgress<NID, bool, bool, QS>,
}

impl<NID, QS> fmt::Display for Voting<NID, QS>
where
    NID: NodeId,
    QS: QuorumSet<NID> + fmt::Debug + 'static,
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

impl<NID, QS> Voting<NID, QS>
where
    NID: NodeId,
    QS: QuorumSet<NID> + fmt::Debug + 'static,
{
    pub(crate) fn new(
        starting_time: Instant,
        vote: Vote<NID>,
        last_log_id: Option<LogId<NID>>,
        quorum_set: QS,
    ) -> Self {
        Self {
            starting_time,
            vote,
            last_log_id,
            progress: VecProgress::new(quorum_set, [], false),
        }
    }

    pub(crate) fn vote_ref(&self) -> &Vote<NID> {
        &self.vote
    }

    pub(crate) fn progress(&self) -> &VecProgress<NID, bool, bool, QS> {
        &self.progress
    }

    /// Grant the vote by a node.
    pub(crate) fn grant_by(&mut self, target: &NID) -> bool {
        let granted = *self.progress.update(target, true).expect("target not in quorum set");

        tracing::info!(voting = debug(&self), "{}", func_name!());

        granted
    }

    /// Return the node ids that has granted this vote.
    #[allow(dead_code)]
    pub(crate) fn granters(&self) -> impl Iterator<Item = NID> + '_ {
        self.progress().iter().filter(|(_, granted)| *granted).map(|(target, _)| *target)
    }
}
