use std::cmp::Ordering;

use openraft_macros::since;

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
#[since]
#[derive(Debug)]
pub enum Metric<C>
where C: RaftTypeConfig
{
    /// The current term, [`RaftMetrics::current_term`]: the term of this node's current vote.
    ///
    /// When this node changes its vote, this term follows the new vote at once, before storage
    /// persists it. It can thus be newer than the term of the vote that [`Vote`](Self::Vote)
    /// observes.
    Term(C::Term),
    /// The last flushed vote, [`RaftMetrics::vote`]: the newest vote that
    /// [`RaftLogStorage::save_vote()`] has persisted.
    ///
    /// When this node changes its vote, `RaftMetrics::vote` keeps the previous vote until
    /// storage persists the new one, while [`Term`](Self::Term) already observes the term of the
    /// new vote.
    ///
    /// Equality and ordering against [`RaftMetrics`] both compare only the leader ID and whether
    /// the vote is committed. Other fields of a custom [`RaftVote`](crate::vote::RaftVote) type,
    /// such as storage metadata, are ignored.
    ///
    /// [`RaftLogStorage::save_vote()`]: crate::storage::RaftLogStorage::save_vote
    #[since(version = "0.10.2", change = "equality compares only the leader ID and commitment")]
    Vote(VoteOf<C>),
    /// The index of the last log entry.
    LastLogIndex(Option<u64>),
    /// The local committed log ID, [`RaftMetrics::local_committed`]: the last log ID that this
    /// node knows to be committed and that is safe to apply to its state machine.
    ///
    /// The local committed log ID can lag [`RaftMetrics::cluster_committed`], the last log ID
    /// known to be committed by a quorum, while this node has not yet received the log entries
    /// up to that log ID. See [`commit`] for the distinction.
    ///
    /// [`commit`]: crate::docs::protocol::commit
    #[since(version = "0.10.0")]
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
            Metric::Vote(v) => self.vote.as_ref_vote() == v.as_ref_vote(),
            Metric::LastLogIndex(v) => self.last_log_index == *v,
            Metric::Committed(v) => &self.local_committed == v,
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
            Metric::Committed(v) => Some(self.local_committed.cmp(v)),
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
    use crate::metrics::testing::MetaVote;
    use crate::metrics::testing::MetaVoteConfig;

    fn init_metrics() -> RaftMetrics<UTConfig> {
        let mut m = RaftMetrics::new_initial(0);
        m.local_committed = Some(log_id(1, 0, 5));
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

    /// A vote that differs only in a field of the custom vote type is equal as a metric.
    #[test]
    fn test_metric_vote_ignores_custom_vote_fields() {
        let mut m = RaftMetrics::<MetaVoteConfig>::new_initial(1);
        m.vote = MetaVote::new(2, 1, true, 7);

        let want = MetaVote::new(2, 1, true, 8);
        assert_ne!(m.vote, want, "the custom vote type compares `meta` too");

        assert_eq!(m, Metric::Vote(want.clone()));
        assert_eq!(m.partial_cmp(&Metric::Vote(want)), Some(Ordering::Equal));
    }

    /// A vote with another leader ID or commitment is not equal, and compares as votes do.
    #[test]
    fn test_metric_vote_compares_leader_and_commitment() {
        let mut m = RaftMetrics::<MetaVoteConfig>::new_initial(1);
        m.vote = MetaVote::new(2, 1, false, 7);

        let cases = [
            ("higher term", MetaVote::new(3, 1, false, 7), Some(Ordering::Less)),
            ("lower term", MetaVote::new(1, 1, false, 7), Some(Ordering::Greater)),
            ("committed", MetaVote::new(2, 1, true, 7), Some(Ordering::Less)),
            ("same term, other node", MetaVote::new(2, 2, false, 7), None),
        ];

        for (change, want, ordering) in cases {
            assert_ne!(m, Metric::Vote(want.clone()), "{change}");
            assert_eq!(m.partial_cmp(&Metric::Vote(want)), ordering, "{change}");
        }
    }
}
