use std::cmp::Ordering;

use crate::LogIdOptionExt;
use crate::RaftMetrics;
use crate::RaftTypeConfig;
use crate::metrics::metric_display::MetricDisplay;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;
use crate::vote::raft_vote::RaftVoteExt;

/// A metric entry of a Raft node.
///
/// This is used to specify which metric to observe.
#[derive(Debug)]
pub enum Metric<C>
where C: RaftTypeConfig
{
    /// The current term.
    Term(C::Term),
    /// The current vote state.
    Vote(VoteOf<C>),
    /// The index of the last log entry.
    LastLogIndex(Option<u64>),
    /// The last committed log ID (accepted by a quorum).
    Committed(Option<LogIdOf<C>>),
    /// The last applied log ID.
    Applied(Option<LogIdOf<C>>),
    /// The index of the last applied log entry.
    AppliedIndex(Option<u64>),
    /// The last snapshot log ID.
    Snapshot(Option<LogIdOf<C>>),
    /// The last purged log ID.
    Purged(Option<LogIdOf<C>>),
}

impl<C> Metric<C>
where C: RaftTypeConfig
{
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Metric::Term(_) => "term",
            Metric::Vote(_) => "vote",
            Metric::LastLogIndex(_) => "last_log_index",
            Metric::Committed(_) => "committed",
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
            Metric::Committed(v) => &self.committed == v,
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
            Metric::Vote(v) => self.vote.as_ref_vote().partial_cmp(&v.as_ref_vote()),
            Metric::LastLogIndex(v) => Some(self.last_log_index.cmp(v)),
            Metric::Committed(v) => Some(self.committed.cmp(v)),
            Metric::Applied(v) => Some(self.last_applied.cmp(v)),
            Metric::AppliedIndex(v) => Some(self.last_applied.index().cmp(v)),
            Metric::Snapshot(v) => Some(self.snapshot.cmp(v)),
            Metric::Purged(v) => Some(self.purged.cmp(v)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use crate::RaftMetrics;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::metrics::Metric;

    fn init_metrics() -> RaftMetrics<UTConfig> {
        let mut m = RaftMetrics::new_initial(0);
        m.committed = Some(log_id(1, 0, 5));
        m
    }

    #[test]
    fn test_metric_committed_name() {
        let m: Metric<UTConfig> = Metric::Committed(None);
        assert_eq!(m.name(), "committed");
    }

    #[test]
    fn test_metric_committed_partial_eq() {
        let m = init_metrics();

        assert_eq!(m, Metric::Committed(Some(log_id(1, 0, 5))));
        assert_ne!(m, Metric::Committed(Some(log_id(1, 0, 4))));
        assert_ne!(m, Metric::Committed(None));
    }

    #[test]
    fn test_metric_committed_partial_ord() {
        let m = init_metrics();

        assert_eq!(
            m.partial_cmp(&Metric::Committed(Some(log_id(1, 0, 5)))),
            Some(Ordering::Equal)
        );
        assert_eq!(
            m.partial_cmp(&Metric::Committed(Some(log_id(1, 0, 3)))),
            Some(Ordering::Greater)
        );
        assert_eq!(
            m.partial_cmp(&Metric::Committed(Some(log_id(2, 0, 6)))),
            Some(Ordering::Less)
        );
        assert_eq!(m.partial_cmp(&Metric::Committed(None)), Some(Ordering::Greater));
    }
}
