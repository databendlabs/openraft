use std::collections::VecDeque;
use std::collections::vec_deque;
use std::fmt;

/// Trait bound for keys in a [`RangeMap`].
pub trait RangeMapKey: Copy + fmt::Debug + fmt::Display + Ord {}
impl<T> RangeMapKey for T where T: Copy + fmt::Debug + fmt::Display + Ord {}

/// Trait bound for values in a [`RangeMap`].
pub trait RangeMapValue: Copy + fmt::Debug {}
impl<T> RangeMapValue for T where T: Copy + fmt::Debug {}

/// A fixed-capacity range map backed by a ring buffer.
///
/// Each entry `(k, v)` represents the range `[prev_entry.k, k)` with value `v`,
/// where the first entry uses an externally tracked `begin` as its left boundary.
///
/// Pre-allocates exactly `capacity` slots and never reallocates.
/// When full, the oldest entry is evicted on append and its key is returned.
///
/// Lookup is O(log n) via binary search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RangeMap<K, V>
where
    K: RangeMapKey,
    V: RangeMapValue,
{
    entries: VecDeque<(K, V)>,
}

impl<K, V> fmt::Display for RangeMap<K, V>
where
    K: RangeMapKey,
    V: RangeMapValue,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{")?;
        for (i, (k, v)) in self.entries.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "({}, {:?})", k, v)?;
        }
        write!(f, "}}")
    }
}

#[allow(dead_code)]
impl<K, V> RangeMap<K, V>
where
    K: RangeMapKey,
    V: RangeMapValue,
{
    /// Create a new ring buffer with the given capacity.
    pub(crate) fn new(capacity: usize) -> Self {
        debug_assert!(capacity > 0, "capacity must be > 0");
        Self {
            entries: VecDeque::with_capacity(capacity),
        }
    }

    /// Record a new range ending at `right_boundary` (exclusive) with `value`.
    ///
    /// Returns the evicted entry's key if the buffer was at capacity, `None` otherwise.
    /// The caller is responsible for advancing its tracked `begin` accordingly.
    pub(crate) fn record(&mut self, right_boundary: K, value: V) -> Option<K> {
        if let Some(&(prev, _)) = self.entries.back() {
            debug_assert!(
                right_boundary > prev,
                "RangeMap::record: right_boundary {:?} must be > previous {:?}",
                right_boundary,
                prev
            );
        }

        let evicted = if self.entries.len() == self.entries.capacity() {
            self.entries.pop_front().map(|(k, _)| k)
        } else {
            None
        };

        self.entries.push_back((right_boundary, value));

        debug_assert!(
            self.entries.len() <= self.entries.capacity(),
            "RangeMap: len {} exceeded capacity {}",
            self.entries.len(),
            self.entries.capacity()
        );

        evicted
    }

    /// Look up the value for the range containing `query`.
    ///
    /// Finds the smallest right boundary greater than `query`.
    /// Returns `None` if empty or `query` is past the last entry.
    ///
    /// O(log n) via binary search.
    pub(crate) fn lookup(&self, query: &K) -> Option<&V> {
        let pos = self.entries.partition_point(|(k, _)| *k <= *query);
        self.entries.get(pos).map(|(_, v)| v)
    }

    /// The exclusive upper bound of the last range, or `None` if empty.
    pub(crate) fn end(&self) -> Option<K> {
        self.entries.back().map(|(k, _)| *k)
    }

    /// Iterate over entries as `(right_boundary, value)` pairs.
    ///
    /// Each entry `(k, v)` represents the range `[prev_key, k)` with value `v`.
    pub(crate) fn entries(&self) -> vec_deque::Iter<'_, (K, V)> {
        self.entries.iter()
    }

    /// Iterate over entries whose right boundary is greater than `begin`.
    ///
    /// Skips entries with right boundary `<= begin`, returning only those
    /// that cover or follow `begin`.
    pub(crate) fn entries_after(&self, begin: K) -> vec_deque::Iter<'_, (K, V)> {
        let start = self.entries.partition_point(|(k, _)| *k <= begin);
        self.entries.range(start..)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // entries: [(10, 100), (20, 200), (30, 300)], begin tracked externally as 0
    // [0, 10) → 100, [10, 20) → 200, [20, 30) → 300
    fn make_buffer() -> RangeMap<u64, u64> {
        let mut rm = RangeMap::new(10);
        rm.record(10, 100);
        rm.record(20, 200);
        rm.record(30, 300);
        rm
    }

    #[test]
    fn test_record_and_lookup() {
        let rm = make_buffer();

        // [0, 10) → 100
        assert_eq!(rm.lookup(&0), Some(&100));
        assert_eq!(rm.lookup(&5), Some(&100));
        assert_eq!(rm.lookup(&9), Some(&100));

        // [10, 20) → 200
        assert_eq!(rm.lookup(&10), Some(&200));
        assert_eq!(rm.lookup(&15), Some(&200));

        // [20, 30) → 300
        assert_eq!(rm.lookup(&20), Some(&300));
        assert_eq!(rm.lookup(&29), Some(&300));

        // Out of range
        assert_eq!(rm.lookup(&30), None);
        assert_eq!(rm.lookup(&100), None);
    }

    #[test]
    fn test_record_returns_none_when_not_full() {
        let mut rm = RangeMap::new(5);
        assert_eq!(rm.record(10, 100), None);
        assert_eq!(rm.record(20, 200), None);
    }

    #[test]
    fn test_eviction_at_capacity() {
        let mut rm = RangeMap::new(2);
        let mut begin = 0u64;

        assert_eq!(rm.record(10, 100), None);
        assert_eq!(rm.record(20, 200), None);
        // At capacity(2), evicts (10, 100)
        assert_eq!(rm.record(30, 300), Some(10));
        begin = begin.max(10);

        // Remaining: [(20, 200), (30, 300)], begin = 10
        assert_eq!(begin, 10);
        // Before first entry's right boundary: still finds value
        assert_eq!(rm.lookup(&5), Some(&200));
        assert_eq!(rm.lookup(&10), Some(&200));
        assert_eq!(rm.lookup(&15), Some(&200));
        assert_eq!(rm.lookup(&20), Some(&300));
        assert_eq!(rm.lookup(&30), None);
    }

    #[test]
    fn test_end_entries() {
        let rm = make_buffer();
        assert_eq!(rm.end(), Some(30));
        let e: Vec<(u64, u64)> = rm.entries().copied().collect();
        assert_eq!(e, vec![(10, 100), (20, 200), (30, 300)]);
    }

    #[test]
    fn test_empty() {
        let rm = RangeMap::<u64, u64>::new(5);
        assert_eq!(rm.lookup(&0), None);
        assert_eq!(rm.lookup(&1), None);
        assert_eq!(rm.end(), None);
        assert_eq!(rm.entries().count(), 0);
    }

    #[test]
    fn test_single_entry() {
        let mut rm = RangeMap::new(5);
        rm.record(10, 100);
        // entries: [(10, 100)]
        assert_eq!(rm.lookup(&0), Some(&100));
        assert_eq!(rm.lookup(&9), Some(&100));
        assert_eq!(rm.lookup(&10), None);
    }

    #[test]
    fn test_display_empty() {
        let rm = RangeMap::<u64, u64>::new(5);
        assert_eq!(format!("{}", rm), "{}");
    }

    #[test]
    fn test_display_entries() {
        let rm = make_buffer();
        assert_eq!(format!("{}", rm), "{(10, 100), (20, 200), (30, 300)}");
    }

    #[test]
    fn test_display_after_eviction() {
        let mut rm = RangeMap::new(2);
        rm.record(10, 100);
        rm.record(20, 200);
        rm.record(30, 300);
        assert_eq!(format!("{}", rm), "{(20, 200), (30, 300)}");
    }

    #[test]
    #[should_panic(expected = "must be >")]
    fn test_non_monotonic_panics() {
        let mut rm = RangeMap::new(5);
        rm.record(20, 100);
        rm.record(15, 200);
    }

    #[test]
    fn test_entries_after() {
        let rm = make_buffer();
        // entries: [(10, 100), (20, 200), (30, 300)]

        // After 0: all entries
        let e: Vec<_> = rm.entries_after(0).copied().collect();
        assert_eq!(e, vec![(10, 100), (20, 200), (30, 300)]);

        // After 10: skip first entry
        let e: Vec<_> = rm.entries_after(10).copied().collect();
        assert_eq!(e, vec![(20, 200), (30, 300)]);

        // After 15: skip first two entries (15 < 20)
        let e: Vec<_> = rm.entries_after(15).copied().collect();
        assert_eq!(e, vec![(20, 200), (30, 300)]);

        // After 20: skip first two entries
        let e: Vec<_> = rm.entries_after(20).copied().collect();
        assert_eq!(e, vec![(30, 300)]);

        // After 30: nothing
        let e: Vec<_> = rm.entries_after(30).copied().collect();
        assert_eq!(e, vec![]);

        // After 100: nothing
        let e: Vec<_> = rm.entries_after(100).copied().collect();
        assert_eq!(e, vec![]);
    }

    #[test]
    fn test_entries_after_empty() {
        let rm = RangeMap::<u64, u64>::new(5);
        let e: Vec<_> = rm.entries_after(0).copied().collect();
        assert_eq!(e, vec![]);
    }

    #[test]
    fn test_string_keys() {
        let mut rm = RangeMap::new(10);
        rm.record("d", 1u32);
        rm.record("g", 2);
        rm.record("m", 3);

        // [_, d) → 1, [d, g) → 2, [g, m) → 3
        assert_eq!(rm.lookup(&"b"), Some(&1));
        assert_eq!(rm.lookup(&"d"), Some(&2));
        assert_eq!(rm.lookup(&"j"), Some(&3));
        assert_eq!(rm.lookup(&"m"), None);
    }
}
