use std::fmt;

use tokio::time::Instant;

use crate::display_ext::DisplayOptionExt;
use crate::progress::Progress;
use crate::progress::VecProgress;
use crate::quorum::QuorumSet;
use crate::utime::UTime;
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
    /// The vote to request the voters to grant.
    vote: UTime<Vote<NID>>,

    /// The last log id when the voting process is started.
    last_log_id: Option<LogId<NID>>,

    /// Which nodes have granted the the vote at certain time point.
    progress: VecProgress<NID, Option<Instant>, Option<Instant>, QS>,
}

impl<NID, QS> fmt::Display for Voting<NID, QS>
where
    NID: NodeId,
    QS: QuorumSet<NID> + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{vote: {}, last_log: {}, progress:{:?} }}",
            self.vote,
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
            vote: UTime::new(starting_time, vote),
            last_log_id,
            progress: VecProgress::new(quorum_set, [], None),
        }
    }

    /// Grant the vote by a node.
    pub(crate) fn grant_by(&mut self, target: &NID) -> bool {
        let granted = *self.progress.update(target, self.vote.utime()).expect("target not in quorum set");

        tracing::info!(voting = debug(&self), "{}", func_name!());

        granted.is_some()
    }

    /// Return the node ids that has granted this vote.
    #[allow(dead_code)]
    pub(crate) fn granters(&self) -> impl Iterator<Item = NID> + '_ {
        self.progress.iter().filter(|(_target, granted)| granted.is_some()).map(|(target, _)| *target)
    }
}
