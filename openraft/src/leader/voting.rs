use std::fmt;

use tokio::time::Instant;

use crate::progress::Progress;
use crate::progress::VecProgress;
use crate::quorum::QuorumSet;
use crate::utime::UTime;
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
    /// The vote to request the voters to grant.
    vote: UTime<Vote<NID>>,

    /// Which nodes have granted the the vote of this node.
    progress: VecProgress<NID, bool, bool, QS>,
}

impl<NID, QS> fmt::Display for Voting<NID, QS>
where
    NID: NodeId,
    QS: QuorumSet<NID> + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{vote: {}, progress:{} }}", self.vote, self.progress)
    }
}

impl<NID, QS> Voting<NID, QS>
where
    NID: NodeId,
    QS: QuorumSet<NID> + 'static,
{
    pub(crate) fn new(starting_time: Instant, vote: Vote<NID>, quorum_set: QS) -> Self {
        Self {
            vote: UTime::new(starting_time, vote),
            progress: VecProgress::new(quorum_set, [], false),
        }
    }

    /// Grant the vote by a node.
    pub(crate) fn grant_by(&mut self, target: &NID) -> bool {
        let granted = *self.progress.update(target, true).expect("target not in quorum set");

        tracing::info!(voting = display(&self), "{}", func_name!());

        granted
    }

    /// Return the node ids that has granted this vote.
    #[allow(dead_code)]
    pub(crate) fn granters(&self) -> impl Iterator<Item = NID> + '_ {
        self.progress.iter().filter(|(_target, granted)| *granted).map(|(target, _)| *target)
    }
}
