use crate::RaftTypeConfig;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;

/// Iterator over log IDs in a [`LeaderLogIds`](super::leader_log_ids::LeaderLogIds) range.
///
/// Uses half-open range `[start, end)` where `start` is inclusive and `end` is exclusive.
/// When `start == end`, the iterator is exhausted.
/// Implements `DoubleEndedIterator` and `ExactSizeIterator`.
#[derive(Debug, Clone)]
pub(crate) struct LeaderLogIdsIter<C: RaftTypeConfig> {
    committed_leader_id: CommittedLeaderIdOf<C>,

    /// Next index to yield from the front (inclusive).
    start: u64,

    /// One past the last index to yield (exclusive).
    end: u64,
}

impl<C: RaftTypeConfig> LeaderLogIdsIter<C> {
    pub(crate) fn new(committed_leader_id: CommittedLeaderIdOf<C>, start: u64, end: u64) -> Self {
        Self {
            committed_leader_id,
            start,
            end,
        }
    }

    fn len(&self) -> usize {
        if self.start >= self.end {
            0
        } else {
            (self.end - self.start) as usize
        }
    }
}

impl<C: RaftTypeConfig> Iterator for LeaderLogIdsIter<C> {
    type Item = LogIdOf<C>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start >= self.end {
            return None;
        }

        let index = self.start;
        self.start += 1;
        Some(LogIdOf::<C>::new(self.committed_leader_id.clone(), index))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<C: RaftTypeConfig> DoubleEndedIterator for LeaderLogIdsIter<C> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.start >= self.end {
            return None;
        }

        self.end -= 1;
        Some(LogIdOf::<C>::new(self.committed_leader_id.clone(), self.end))
    }
}

impl<C: RaftTypeConfig> ExactSizeIterator for LeaderLogIdsIter<C> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::type_config::alias::CommittedLeaderIdOf;

    fn committed_leader_id(term: u64, node_id: u64) -> CommittedLeaderIdOf<UTConfig> {
        *log_id(term, node_id, 0).committed_leader_id()
    }

    #[test]
    fn test_empty_range() {
        // start == end means empty
        let iter = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 5, 5);
        assert_eq!(iter.len(), 0);
        let ids: Vec<_> = iter.collect();
        assert!(ids.is_empty());
    }

    #[test]
    fn test_single_element() {
        // [0, 1) is a single element at index 0
        let iter = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 0, 1);
        assert_eq!(iter.len(), 1);
        let ids: Vec<_> = iter.collect();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].index(), 0);
    }

    #[test]
    fn test_forward_iteration() {
        let iter = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 5, 8);
        let ids: Vec<_> = iter.collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0].index(), 5);
        assert_eq!(ids[1].index(), 6);
        assert_eq!(ids[2].index(), 7);
    }

    #[test]
    fn test_backward_iteration() {
        let mut iter = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 5, 8);

        assert_eq!(iter.next_back().unwrap().index(), 7);
        assert_eq!(iter.next_back().unwrap().index(), 6);
        assert_eq!(iter.next_back().unwrap().index(), 5);
        assert!(iter.next_back().is_none());
    }

    #[test]
    fn test_mixed_iteration() {
        let mut iter = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 5, 8);

        assert_eq!(iter.next().unwrap().index(), 5);
        assert_eq!(iter.next_back().unwrap().index(), 7);
        assert_eq!(iter.next().unwrap().index(), 6);
        assert!(iter.next().is_none());
        assert!(iter.next_back().is_none());
    }

    #[test]
    fn test_start_at_zero_forward() {
        let iter = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 0, 3);
        let ids: Vec<_> = iter.collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0].index(), 0);
        assert_eq!(ids[1].index(), 1);
        assert_eq!(ids[2].index(), 2);
    }

    #[test]
    fn test_start_at_zero_backward() {
        let mut iter = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 0, 3);

        assert_eq!(iter.next_back().unwrap().index(), 2);
        assert_eq!(iter.next_back().unwrap().index(), 1);
        assert_eq!(iter.next_back().unwrap().index(), 0);
        assert!(iter.next_back().is_none());
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_single_element_at_zero() {
        let mut iter = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 0, 1);

        let elem = iter.next_back().unwrap();
        assert_eq!(elem.index(), 0);

        assert!(iter.next_back().is_none());
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_exact_size() {
        let iter = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 10, 15);
        assert_eq!(iter.len(), 5);

        let mut iter = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 10, 15);
        assert_eq!(iter.len(), 5);
        iter.next();
        assert_eq!(iter.len(), 4);
        iter.next_back();
        assert_eq!(iter.len(), 3);
    }

    #[test]
    fn test_clone() {
        let iter1 = LeaderLogIdsIter::<UTConfig>::new(committed_leader_id(1, 1), 5, 8);
        let iter2 = iter1.clone();

        let ids1: Vec<_> = iter1.collect();
        let ids2: Vec<_> = iter2.collect();
        assert_eq!(ids1, ids2);
    }
}
