use std::fmt;

use crate::RaftTypeConfig;
use crate::engine::leader_log_ids_iter::LeaderLogIdsIter;
use crate::log_id::ref_log_id::RefLogId;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;

/// A non-empty range of log IDs belonging to a Leader.
///
/// This struct represents a contiguous range of log IDs that share the same
/// committed leader ID. It provides efficient access to the first and last
/// log IDs, and can be iterated over to produce all log IDs in the range.
///
/// The range is `[first, last]` (both inclusive).
/// This struct always represents a non-empty range; use `Option<LeaderLogIds>`
/// when an empty range is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeaderLogIds<C: RaftTypeConfig> {
    committed_leader_id: CommittedLeaderIdOf<C>,

    /// First index (inclusive).
    first: u64,

    /// Last index (inclusive).
    last: u64,
}

impl<C> fmt::Display for LeaderLogIds<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:[{}, {}]", self.committed_leader_id, self.first, self.last)
    }
}

impl<C> LeaderLogIds<C>
where C: RaftTypeConfig
{
    /// Create a new log ID range `[first, last]`.
    pub(crate) fn new(committed_leader_id: CommittedLeaderIdOf<C>, first: u64, last: u64) -> Self {
        debug_assert!(first <= last, "first {} must be <= last {}", first, last);
        Self {
            committed_leader_id,
            first,
            last,
        }
    }

    pub(crate) fn new_single(log_id: LogIdOf<C>) -> Self {
        let index = log_id.index();
        Self {
            committed_leader_id: log_id.committed_leader_id().clone(),
            first: index,
            last: index,
        }
    }

    /// Returns the first log ID in the range.
    #[allow(dead_code)]
    pub(crate) fn first_log_id(&self) -> LogIdOf<C> {
        LogIdOf::<C>::new(self.committed_leader_id.clone(), self.first)
    }

    /// Returns a reference to the first log ID in the range.
    pub(crate) fn first_ref(&self) -> RefLogId<'_, C> {
        RefLogId::new(&self.committed_leader_id, self.first)
    }

    /// Returns the last log ID in the range.
    #[allow(dead_code)]
    pub(crate) fn last_log_id(&self) -> LogIdOf<C> {
        LogIdOf::<C>::new(self.committed_leader_id.clone(), self.last)
    }

    /// Returns a reference to the last log ID in the range.
    pub(crate) fn last_ref(&self) -> RefLogId<'_, C> {
        RefLogId::new(&self.committed_leader_id, self.last)
    }

    /// Returns the log ID at the specified index.
    ///
    /// # Panics
    /// Panics if `index` is out of range `[first, last]`.
    #[allow(dead_code)]
    pub(crate) fn get(&self, index: u64) -> LogIdOf<C> {
        debug_assert!(
            index >= self.first && index <= self.last,
            "index {} out of range [{}, {}]",
            index,
            self.first,
            self.last
        );
        LogIdOf::<C>::new(self.committed_leader_id.clone(), index)
    }

    /// Returns a reference to the log ID at the specified index.
    ///
    /// # Panics
    /// Panics if `index` is out of range `[first, last]`.
    #[allow(dead_code)]
    pub(crate) fn ref_at(&self, index: u64) -> RefLogId<'_, C> {
        debug_assert!(
            index >= self.first && index <= self.last,
            "index {} out of range [{}, {}]",
            index,
            self.first,
            self.last
        );
        RefLogId::new(&self.committed_leader_id, index)
    }

    /// Returns the number of log IDs in the range.
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        if self.first > self.last {
            0
        } else {
            (self.last - self.first + 1) as usize
        }
    }
}

impl<C> IntoIterator for LeaderLogIds<C>
where C: RaftTypeConfig
{
    type Item = LogIdOf<C>;
    type IntoIter = LeaderLogIdsIter<C>;

    fn into_iter(self) -> Self::IntoIter {
        LeaderLogIdsIter::new(
            self.committed_leader_id,
            self.first,
            self.last + 1, // half-open: [start, end)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;

    fn committed_leader_id(term: u64, node_id: u64) -> CommittedLeaderIdOf<UTConfig> {
        *log_id(term, node_id, 0).committed_leader_id()
    }

    #[test]
    fn test_single_element() {
        // [5, 5] is a single element range
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(1, 1), 5, 5);
        assert_eq!(range.len(), 1);

        let first = range.first_log_id();
        let last = range.last_log_id();
        assert_eq!(first.index(), 5);
        assert_eq!(last.index(), 5);
        assert_eq!(first, last);
    }

    #[test]
    fn test_multiple_elements() {
        // [10, 14] is 5 elements
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(2, 1), 10, 14);
        assert_eq!(range.len(), 5);

        let first = range.first_log_id();
        let last = range.last_log_id();
        assert_eq!(first.index(), 10);
        assert_eq!(last.index(), 14);
    }

    #[test]
    fn test_iterator() {
        // [5, 7] is 3 elements: 5, 6, 7
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(1, 1), 5, 7);
        let ids: Vec<_> = range.into_iter().collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0].index(), 5);
        assert_eq!(ids[1].index(), 6);
        assert_eq!(ids[2].index(), 7);
    }

    #[test]
    fn test_double_ended_iterator() {
        // [5, 7] is 3 elements: 5, 6, 7
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(1, 1), 5, 7);
        let mut iter = range.into_iter();

        let last = iter.next_back().unwrap();
        assert_eq!(last.index(), 7);

        let first = iter.next().unwrap();
        assert_eq!(first.index(), 5);

        let middle = iter.next().unwrap();
        assert_eq!(middle.index(), 6);

        assert!(iter.next().is_none());
        assert!(iter.next_back().is_none());
    }

    #[test]
    fn test_clone_iter() {
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(1, 1), 5, 7);
        let iter1 = range.into_iter();
        let iter2 = iter1.clone();

        let ids1: Vec<_> = iter1.collect();
        let ids2: Vec<_> = iter2.collect();
        assert_eq!(ids1, ids2);
    }

    #[test]
    fn test_iterator_starting_at_zero() {
        // Edge case: range starting at index 0
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(1, 1), 0, 2);
        let mut iter = range.into_iter();

        // Consume from back first to test the edge case
        let last = iter.next_back().unwrap();
        assert_eq!(last.index(), 2);

        let mid = iter.next_back().unwrap();
        assert_eq!(mid.index(), 1);

        let first = iter.next_back().unwrap();
        assert_eq!(first.index(), 0);

        // Iterator should be exhausted
        assert!(iter.next_back().is_none());
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_single_element_at_zero() {
        // Edge case: single element at index 0
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(1, 1), 0, 0);
        let mut iter = range.into_iter();

        let elem = iter.next_back().unwrap();
        assert_eq!(elem.index(), 0);

        assert!(iter.next_back().is_none());
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_new_single() {
        let range = LeaderLogIds::<UTConfig>::new_single(log_id(1, 2, 5));
        assert_eq!(range.len(), 1);
        assert_eq!(range.first_log_id().index(), 5);
        assert_eq!(range.last_log_id().index(), 5);
    }

    #[test]
    fn test_display() {
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(1, 1), 5, 7);
        assert_eq!(format!("{}", range), "T1-N1:[5, 7]");
    }

    #[test]
    fn test_first_ref() {
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(1, 2), 5, 10);
        let first = range.first_ref();
        assert_eq!(first.index(), 5);
        assert_eq!(*first.committed_leader_id(), committed_leader_id(1, 2));
        assert_eq!(first.into_log_id(), log_id(1, 2, 5));
    }

    #[test]
    fn test_last_ref() {
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(1, 2), 5, 10);
        let last = range.last_ref();
        assert_eq!(last.index(), 10);
        assert_eq!(*last.committed_leader_id(), committed_leader_id(1, 2));
        assert_eq!(last.into_log_id(), log_id(1, 2, 10));
    }

    #[test]
    fn test_ref_at() {
        let range = LeaderLogIds::<UTConfig>::new(committed_leader_id(1, 2), 5, 10);

        // At first
        let r = range.ref_at(5);
        assert_eq!(r.index(), 5);
        assert_eq!(r.into_log_id(), log_id(1, 2, 5));

        // In middle
        let r = range.ref_at(7);
        assert_eq!(r.index(), 7);
        assert_eq!(r.into_log_id(), log_id(1, 2, 7));

        // At last
        let r = range.ref_at(10);
        assert_eq!(r.index(), 10);
        assert_eq!(r.into_log_id(), log_id(1, 2, 10));
    }
}
