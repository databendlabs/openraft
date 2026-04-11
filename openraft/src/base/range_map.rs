use std::collections::VecDeque;
use std::collections::vec_deque;
use std::fmt;

/// Trait bound for boundaries in a [`RangeMap`].
pub trait RangeMapBound: Copy + fmt::Debug + fmt::Display + Ord {}
impl<T> RangeMapBound for T where T: Copy + fmt::Debug + fmt::Display + Ord {}

/// Trait bound for values in a [`RangeMap`].
pub trait RangeMapValue: Copy + fmt::Debug {}
impl<T> RangeMapValue for T where T: Copy + fmt::Debug {}

/// A fixed-capacity range map backed by a ring buffer.
///
/// Each entry `(k, v)` represents the range `[prev_entry.k, k)` with value `v`;
/// the first entry uses an externally tracked `begin` as its left boundary.
///
/// Pre-allocates `capacity` slots. When full, the oldest entry is evicted on
/// append and its boundary is returned. A non-monotonic right boundary
/// rewrites the tail in place — see [`RangeMap::record`].
///
/// Lookup is O(log n) via binary search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RangeMap<Bound, V>
where
    Bound: RangeMapBound,
    V: RangeMapValue,
{
    entries: VecDeque<(Bound, V)>,
}

impl<Bound, V> fmt::Display for RangeMap<Bound, V>
where
    Bound: RangeMapBound,
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

impl<Bound, V> RangeMap<Bound, V>
where
    Bound: RangeMapBound,
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
    /// If `right_boundary` is strictly greater than the current last boundary,
    /// the range is appended. Otherwise, trailing entries with boundary
    /// `>= right_boundary` are popped first, then the new range is pushed —
    /// rewriting the tail so the latest recording wins on the overlapping
    /// suffix. This supports a range space that can be revised downwards, e.g.
    /// a Raft log index re-appended under a new term after truncation.
    ///
    /// Returns the front entry's key if this insertion evicted the oldest
    /// entry, `None` otherwise. Entries popped from the back by tail rewriting
    /// are **not** evictions and are never returned — the caller's `begin`
    /// should only advance on front eviction.
    pub(crate) fn record(&mut self, right_boundary: Bound, value: V) -> Option<Bound> {
        // Rewrite the tail: drop trailing entries whose right boundary is not
        // strictly less than the new one.
        while let Some(&(prev, _)) = self.entries.back() {
            if right_boundary > prev {
                break;
            }
            #[cfg(debug_assertions)]
            tracing::info!("RangeMap::record: drop tail {prev}, new right_boundary {right_boundary} <= it");
            self.entries.pop_back();
        }

        let evicted = if self.entries.len() == self.entries.capacity() {
            self.entries.pop_front().map(|(k, _)| k)
        } else {
            None
        };

        self.entries.push_back((right_boundary, value));

        evicted
    }

    /// Look up the value for the range containing `query`.
    ///
    /// Finds the smallest right boundary greater than `query`.
    /// Returns `None` if empty or `query` is past the last entry.
    ///
    /// O(log n) via binary search.
    #[allow(dead_code)]
    pub(crate) fn lookup(&self, query: &Bound) -> Option<&V> {
        let pos = self.entries.partition_point(|(k, _)| *k <= *query);
        self.entries.get(pos).map(|(_, v)| v)
    }

    /// The exclusive upper bound of the last range, or `None` if empty.
    pub(crate) fn end(&self) -> Option<Bound> {
        self.entries.back().map(|(k, _)| *k)
    }

    /// Iterate over entries as `(right_boundary, value)` pairs.
    ///
    /// Each entry `(k, v)` represents the range `[prev_key, k)` with value `v`.
    #[allow(dead_code)]
    pub(crate) fn entries(&self) -> vec_deque::Iter<'_, (Bound, V)> {
        self.entries.iter()
    }

    /// Iterate over entries whose right boundary is greater than `begin`.
    ///
    /// Skips entries with right boundary `<= begin`, returning only those
    /// that cover or follow `begin`.
    pub(crate) fn entries_after(&self, begin: Bound) -> vec_deque::Iter<'_, (Bound, V)> {
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

    // --- Tail rewriting ------------------------------------------------

    #[test]
    fn test_record_override_same_boundary_replaces_value() {
        // Recording with the same right boundary drops the previous entry
        // and installs the new one.
        let mut rm = RangeMap::new(5);
        rm.record(10, 100);
        assert_eq!(rm.record(10, 200), None);

        let e: Vec<_> = rm.entries().copied().collect();
        assert_eq!(e, vec![(10, 200)]);

        // Lookups reflect the new value only.
        assert_eq!(rm.lookup(&0), Some(&200));
        assert_eq!(rm.lookup(&9), Some(&200));
        assert_eq!(rm.lookup(&10), None);
    }

    #[test]
    fn test_record_override_smaller_boundary_pops_multiple() {
        // entries: [(10, 100), (20, 200), (30, 300)]
        // Recording (15, 400) must pop both (20, 200) and (30, 300), then push (15, 400).
        let mut rm = make_buffer();
        assert_eq!(rm.record(15, 400), None);

        let e: Vec<_> = rm.entries().copied().collect();
        assert_eq!(e, vec![(10, 100), (15, 400)]);

        // Untouched lower range keeps its original value.
        assert_eq!(rm.lookup(&9), Some(&100));
        // Rewritten range uses the new value.
        assert_eq!(rm.lookup(&10), Some(&400));
        assert_eq!(rm.lookup(&14), Some(&400));
        assert_eq!(rm.lookup(&15), None);
    }

    #[test]
    fn test_record_override_preserves_smaller_entries() {
        // entries: [(10, 100), (20, 200), (30, 300)]
        // Recording (21, 400) pops only (30, 300); the earlier entries are untouched.
        let mut rm = make_buffer();
        assert_eq!(rm.record(21, 400), None);

        let e: Vec<_> = rm.entries().copied().collect();
        assert_eq!(e, vec![(10, 100), (20, 200), (21, 400)]);

        assert_eq!(rm.lookup(&5), Some(&100));
        assert_eq!(rm.lookup(&15), Some(&200));
        assert_eq!(rm.lookup(&20), Some(&400));
        assert_eq!(rm.lookup(&21), None);
    }

    #[test]
    fn test_record_override_empties_buffer() {
        // A right boundary smaller than every existing one drops them all.
        let mut rm = make_buffer();
        assert_eq!(rm.record(5, 400), None);

        let e: Vec<_> = rm.entries().copied().collect();
        assert_eq!(e, vec![(5, 400)]);
        assert_eq!(rm.lookup(&4), Some(&400));
        assert_eq!(rm.lookup(&5), None);
    }

    #[test]
    fn test_record_override_does_not_evict_front() {
        // Tail rewriting is not a front eviction: even when the buffer is
        // full, popping from the back must not return an evicted front key.
        let mut rm = RangeMap::new(2);
        rm.record(10, 100);
        rm.record(20, 200);
        // Buffer is full (len == capacity == 2).
        assert_eq!(rm.record(20, 300), None);

        let e: Vec<_> = rm.entries().copied().collect();
        assert_eq!(e, vec![(10, 100), (20, 300)]);
    }

    #[test]
    fn test_record_override_after_front_eviction() {
        // Tail rewriting works correctly after `begin` has been advanced
        // by a prior front eviction.
        let mut rm = RangeMap::new(2);
        let mut begin = 0u64;

        rm.record(10, 100);
        rm.record(20, 200);
        // Evicts (10, 100); caller advances begin to 10.
        assert_eq!(rm.record(30, 300), Some(10));
        begin = begin.max(10);

        // Rewrite the back entry in place. No further eviction must happen.
        assert_eq!(rm.record(30, 400), None);
        assert_eq!(begin, 10);

        let e: Vec<_> = rm.entries().copied().collect();
        assert_eq!(e, vec![(20, 200), (30, 400)]);
        // The rewritten range resolves to the new value.
        assert_eq!(rm.lookup(&25), Some(&400));
    }

    #[test]
    fn test_record_append_after_override() {
        // After a tail rewrite, normal monotonic appends continue to work.
        let mut rm = make_buffer();
        rm.record(15, 400); // entries: [(10, 100), (15, 400)]
        rm.record(25, 500);

        let e: Vec<_> = rm.entries().copied().collect();
        assert_eq!(e, vec![(10, 100), (15, 400), (25, 500)]);
        assert_eq!(rm.end(), Some(25));
    }

    #[test]
    fn test_record_override_on_empty_buffer() {
        // Rewriting on an empty buffer is a no-op loop; the entry is simply pushed.
        let mut rm: RangeMap<u64, u64> = RangeMap::new(5);
        assert_eq!(rm.record(10, 100), None);
        let e: Vec<_> = rm.entries().copied().collect();
        assert_eq!(e, vec![(10, 100)]);
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
