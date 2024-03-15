use std::cmp::Ordering;

use crate::metrics::metric_display::MetricDisplay;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::RaftMetrics;
use crate::RaftTypeConfig;
use crate::Vote;

/// A metric entry of a Raft node.
///
/// This is used to specify which metric to observe.
#[derive(Debug)]
pub enum Metric<C>
where C: RaftTypeConfig
{
    Term(u64),
    Vote(Vote<C::NodeId>),
    LastLogIndex(Option<u64>),
    Applied(Option<LogId<C::NodeId>>),
    AppliedIndex(Option<u64>),
    Snapshot(Option<LogId<C::NodeId>>),
    Purged(Option<LogId<C::NodeId>>),
}

impl<C> Metric<C>
where C: RaftTypeConfig
{
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Metric::Term(_) => "term",
            Metric::Vote(_) => "vote",
            Metric::LastLogIndex(_) => "last_log_index",
            Metric::Applied(_) => "applied",
            Metric::AppliedIndex(_) => "applied_index",
            Metric::Snapshot(_) => "snapshot",
            Metric::Purged(_) => "purged",
        }
    }

    pub(crate) fn value(&self) -> MetricDisplay<'_, C> {
        MetricDisplay { metric: self }
    }
}

/// Metric can be compared with RaftMetrics by comparing the corresponding field of RaftMetrics.
impl<C> PartialEq<Metric<C>> for RaftMetrics<C>
where C: RaftTypeConfig
{
    fn eq(&self, other: &Metric<C>) -> bool {
        match other {
            Metric::Term(v) => self.current_term == *v,
            Metric::Vote(v) => &self.vote == v,
            Metric::LastLogIndex(v) => self.last_log_index == *v,
            Metric::Applied(v) => &self.last_applied == v,
            Metric::AppliedIndex(v) => self.last_applied.index() == *v,
            Metric::Snapshot(v) => &self.snapshot == v,
            Metric::Purged(v) => &self.purged == v,
        }
    }
}

/// Metric can be compared with RaftMetrics by comparing the corresponding field of RaftMetrics.
impl<C> PartialOrd<Metric<C>> for RaftMetrics<C>
where C: RaftTypeConfig
{
    fn partial_cmp(&self, other: &Metric<C>) -> Option<Ordering> {
        match other {
            Metric::Term(v) => Some(self.current_term.cmp(v)),
            Metric::Vote(v) => self.vote.partial_cmp(v),
            Metric::LastLogIndex(v) => Some(self.last_log_index.cmp(v)),
            Metric::Applied(v) => Some(self.last_applied.cmp(v)),
            Metric::AppliedIndex(v) => Some(self.last_applied.index().cmp(v)),
            Metric::Snapshot(v) => Some(self.snapshot.cmp(v)),
            Metric::Purged(v) => Some(self.purged.cmp(v)),
        }
    }
}
