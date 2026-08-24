/// Orders pending reads by response deadline and insertion sequence.
///
/// A read's deadline is independent of its quorum-acknowledgement threshold, because
/// [`LinearizerOption::wait_timeout`] is chosen per request. This key therefore forms a secondary
/// index over the same reads, ordered by the instant at which each read must be answered.
///
/// [`LinearizerOption::wait_timeout`]: crate::raft::linearizable_read::LinearizerOption
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct PendingReadDeadlineKey<I> {
    pub(super) deadline: I,
    pub(super) sequence: u64,
}
