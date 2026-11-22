use super::LeaderLogIdsIter;
use crate::engine::testing::UtClid;
use crate::engine::testing::log_id;

fn committed_leader_id(term: u64, node_id: u64) -> UtClid {
    *log_id(term, node_id, 0).committed_leader_id()
}

/// `start >= end` is an empty range, which stays exhausted from both ends.
#[test]
fn test_empty_range() {
    for (start, end) in [(5, 5), (6, 5)] {
        let mut iter = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 1), start, end);
        assert_eq!(0, ExactSizeIterator::len(&iter), "[{start}, {end})");
        assert_eq!((0, Some(0)), Iterator::size_hint(&iter), "[{start}, {end})");
        assert_eq!(None, iter.next());
        assert_eq!(None, iter.next_back());
        assert_eq!(None, iter.next());
    }
}

#[test]
fn test_single_element() {
    // [0, 1) is a single element at index 0
    let iter = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 2), 0, 1);
    assert_eq!(1, ExactSizeIterator::len(&iter));
    let ids: Vec<_> = iter.collect();
    assert_eq!(ids, vec![log_id(1, 2, 0)]);
}

#[test]
fn test_forward_iteration() {
    let iter = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 2), 5, 8);
    let ids: Vec<_> = iter.collect();
    assert_eq!(ids, vec![log_id(1, 2, 5), log_id(1, 2, 6), log_id(1, 2, 7)]);
}

#[test]
fn test_backward_iteration() {
    let mut iter = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 2), 5, 8);

    assert_eq!(iter.next_back(), Some(log_id(1, 2, 7)));
    assert_eq!(iter.next_back(), Some(log_id(1, 2, 6)));
    assert_eq!(iter.next_back(), Some(log_id(1, 2, 5)));
    assert!(iter.next_back().is_none());
}

#[test]
fn test_mixed_iteration() {
    let mut iter = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 2), 5, 8);

    assert_eq!(iter.next(), Some(log_id(1, 2, 5)));
    assert_eq!(iter.next_back(), Some(log_id(1, 2, 7)));
    assert_eq!(iter.next(), Some(log_id(1, 2, 6)));
    assert!(iter.next().is_none());
    assert!(iter.next_back().is_none());
}

#[test]
fn test_start_at_zero_forward() {
    let iter = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 2), 0, 3);
    let ids: Vec<_> = iter.collect();
    assert_eq!(ids, vec![log_id(1, 2, 0), log_id(1, 2, 1), log_id(1, 2, 2)]);
}

#[test]
fn test_start_at_zero_backward() {
    let mut iter = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 2), 0, 3);

    assert_eq!(iter.next_back(), Some(log_id(1, 2, 2)));
    assert_eq!(iter.next_back(), Some(log_id(1, 2, 1)));
    assert_eq!(iter.next_back(), Some(log_id(1, 2, 0)));
    assert!(iter.next_back().is_none());
    assert!(iter.next().is_none());
}

#[test]
fn test_single_element_at_zero() {
    let mut iter = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 2), 0, 1);

    assert_eq!(iter.next_back(), Some(log_id(1, 2, 0)));
    assert!(iter.next_back().is_none());
    assert!(iter.next().is_none());
}

/// `ExactSizeIterator::len()` and `Iterator::size_hint()` both count the IDs left, as either end
/// advances and after the iterator is exhausted.
#[test]
fn test_exact_size() {
    let left = |iter: &LeaderLogIdsIter<UtClid>| (ExactSizeIterator::len(iter), Iterator::size_hint(iter));

    let mut iter = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 1), 10, 14);
    assert_eq!((4, (4, Some(4))), left(&iter));

    assert_eq!(Some(log_id(1, 1, 10)), iter.next());
    assert_eq!((3, (3, Some(3))), left(&iter));

    assert_eq!(Some(log_id(1, 1, 13)), iter.next_back());
    assert_eq!((2, (2, Some(2))), left(&iter));

    assert_eq!(Some(log_id(1, 1, 11)), iter.next());
    assert_eq!((1, (1, Some(1))), left(&iter));

    assert_eq!(Some(log_id(1, 1, 12)), iter.next_back());
    assert_eq!((0, (0, Some(0))), left(&iter));

    assert_eq!(None, iter.next());
    assert_eq!(None, iter.next_back());
    assert_eq!(None, iter.next());
    assert_eq!(None, iter.next_back());
    assert_eq!((0, (0, Some(0))), left(&iter));
}

#[test]
fn test_clone() {
    let iter1 = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 1), 5, 8);
    let iter2 = iter1.clone();

    let ids1: Vec<_> = iter1.collect();
    let ids2: Vec<_> = iter2.collect();
    assert_eq!(ids1, ids2);
}

/// A range at the top of the index space keeps its full log IDs and its exact length from both
/// ends; equal and reversed bounds there are empty.
#[test]
fn test_range_at_u64_max() {
    let clid = committed_leader_id(1, 1);

    let mut iter = LeaderLogIdsIter::<UtClid>::new(clid, u64::MAX - 3, u64::MAX);
    assert_eq!(3, ExactSizeIterator::len(&iter));
    assert_eq!(Some(log_id(1, 1, u64::MAX - 3)), iter.next());
    assert_eq!(Some(log_id(1, 1, u64::MAX - 1)), iter.next_back());
    assert_eq!(1, ExactSizeIterator::len(&iter));
    assert_eq!(Some(log_id(1, 1, u64::MAX - 2)), iter.next_back());
    assert_eq!(0, ExactSizeIterator::len(&iter));
    assert_eq!(None, iter.next());
    assert_eq!(None, iter.next_back());

    for (start, end) in [(u64::MAX, u64::MAX), (u64::MAX, 0), (u64::MAX - 1, 5)] {
        let mut iter = LeaderLogIdsIter::<UtClid>::new(clid, start, end);
        assert_eq!((0, Some(0)), Iterator::size_hint(&iter), "[{start}, {end})");
        assert_eq!(None, iter.next());
        assert_eq!(None, iter.next_back());
    }
}

/// The longest range whose length `usize` can count is accepted with that exact length, checked
/// without traversing it.
#[test]
fn test_span_of_usize_max() {
    let end = u64::try_from(usize::MAX).unwrap();

    let mut iter = LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 1), 0, end);
    assert_eq!(usize::MAX, ExactSizeIterator::len(&iter));
    assert_eq!((usize::MAX, Some(usize::MAX)), Iterator::size_hint(&iter));

    assert_eq!(Some(log_id(1, 1, 0)), iter.next());
    assert_eq!(Some(log_id(1, 1, end - 1)), iter.next_back());
    assert_eq!(usize::MAX - 2, ExactSizeIterator::len(&iter));
}

/// Where `usize` is narrower than `u64`, a longer range is rejected when it is created.
#[cfg(not(target_pointer_width = "64"))]
#[test]
#[should_panic(expected = "more than usize can count")]
fn test_span_over_usize_max() {
    let end = u64::try_from(usize::MAX).unwrap() + 1;
    LeaderLogIdsIter::<UtClid>::new(committed_leader_id(1, 1), 0, end);
}
