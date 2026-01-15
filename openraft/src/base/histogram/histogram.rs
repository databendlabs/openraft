use std::collections::VecDeque;

use super::log_scale::LOG_SCALE;
use super::log_scale::LogScale3;
use super::percentile_stats::PercentileStats;
use super::slot::Slot;

/// A histogram for tracking the distribution of u64 values using logarithmic bucketing.
///
/// This histogram provides O(1) recording and efficient percentile calculation with
/// bounded memory usage (252 buckets = ~2KB per slot), regardless of the number of samples.
///
/// # Multi-Slot Support
///
/// The histogram supports multiple slots for sliding-window metrics. Each slot contains
/// independent bucket counts and optional user-defined metadata. Use `advance()` to rotate
/// to a new slot, which clears the oldest data when the histogram is full.
///
/// # Bucketing Strategy
///
/// Uses logarithmic bucketing where smaller values get higher precision, similar to
/// [HDRHistogram](https://github.com/HdrHistogram/HdrHistogram). The bucket boundaries
/// are determined by the binary representation of the value:
///
/// ```text
/// Group  Bucket   Value Range     Binary Pattern (3-bit window)
/// ─────  ──────   ───────────     ─────────────────────────────
///   0      0-3    [0-3]           Direct mapping (special case)
///   1      4-7    [4-7]           100, 101, 110, 111
///   2     8-11    [8-15]          1xx0, 1xx0 (step=2)
///   3    12-15    [16-31]         1xx00, 1xx00 (step=4)
///   4    16-19    [32-63]         1xx000, 1xx000 (step=8)
///   ...
/// ```
///
/// Each group covers a power-of-2 range and contains 4 buckets. The 2 bits after the
/// MSB determine which bucket within the group:
///
/// ```text
/// Example: value = 42 (binary: 101010)
///   MSB position: 5 (counting from 0)
///   Group: 5 - 2 = 3
///   Bits after MSB: 01 (from 1[01]010)
///   Bucket within group: 1
///   Final bucket index: 4 + (3 * 4) + 1 = 17
/// ```
///
/// # Precision
///
/// - Values 0-7: exact (1:1 mapping)
/// - Values 8-15: ±1 (2 values per bucket)
/// - Values 16-31: ±2 (4 values per bucket)
/// - Values 2^n to 2^(n+1)-1: ±2^(n-2)
///
/// Relative error is bounded at ~12.5% for values >= 8.
///
/// # Memory Usage
///
/// Fixed at 252 buckets * 8 bytes = 2,016 bytes per slot, covering the entire
/// u64 range [0, 2^64-1].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram<T = ()> {
    /// Log scale for value-to-bucket mapping.
    log_scale: &'static LogScale3,

    /// Slots containing bucket counts and metadata. Uses VecDeque for O(1) front removal.
    /// All slots in the deque are active. First slot (index 0) is oldest, last is current.
    /// The VecDeque's capacity determines the maximum number of slots.
    slots: VecDeque<Slot<T>>,

    /// Aggregate bucket counts across all active slots.
    /// Maintained incrementally: +1 on record(), -slot on eviction.
    aggregate_buckets: Vec<u64>,
}

impl<T> Default for Histogram<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Histogram<T> {
    /// Creates a new histogram with 1 slot and 252 buckets.
    ///
    /// Memory usage: 252 * 8 bytes = 2,016 bytes per histogram.
    pub fn new() -> Self {
        Self::with_slots(1)
    }

    /// Creates a new histogram with the specified slot capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of slots.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is 0.
    pub fn with_slots(capacity: usize) -> Self {
        Self::with_log_scale(&LOG_SCALE, capacity)
    }

    /// Creates a new histogram with custom log scale and slot capacity.
    ///
    /// # Arguments
    ///
    /// * `log_scale` - Log scale for value-to-bucket mapping.
    /// * `capacity` - Maximum number of slots.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is 0.
    pub fn with_log_scale(log_scale: &'static LogScale3, capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be at least 1");

        let num_buckets = log_scale.num_buckets();

        let mut slots = VecDeque::with_capacity(capacity);
        slots.push_back(Slot::new(num_buckets));

        Self {
            log_scale,
            slots,
            aggregate_buckets: vec![0; num_buckets],
        }
    }

    /// Records a value to the current (last) slot.
    pub fn record(&mut self, value: u64) {
        let bucket_index = self.log_scale.calculate_bucket(value);
        self.slots.back_mut().unwrap().buckets[bucket_index] += 1;
        self.aggregate_buckets[bucket_index] += 1;
    }

    /// Advances to a new slot, evicting the oldest if at capacity.
    ///
    /// Returns the number of active slots after advancing.
    ///
    /// Logic:
    /// 1. If at capacity, remove the oldest slot (front)
    /// 2. Push a new slot to the back with the given data
    #[allow(dead_code)]
    pub fn advance(&mut self, data: T) -> usize {
        if self.slots.len() == self.slots.capacity() {
            // Subtract evicted slot from aggregate
            let evicted = self.slots.pop_front().unwrap();
            for (i, &count) in evicted.buckets.iter().enumerate() {
                self.aggregate_buckets[i] -= count;
            }
        }

        let mut slot = Slot::new(self.log_scale.num_buckets());
        slot.data = Some(data);
        self.slots.push_back(slot);

        self.slots.len()
    }

    /// Returns the number of active slots.
    #[allow(dead_code)]
    #[inline]
    pub fn active_slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Returns the slot capacity.
    #[allow(dead_code)]
    #[inline]
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    /// Returns a reference to the slot at the given index.
    ///
    /// Index 0 is the oldest slot, index `len - 1` is the current slot.
    /// Returns `None` if the index is out of bounds.
    #[cfg(test)]
    #[inline]
    pub(crate) fn slot(&self, index: usize) -> Option<&Slot<T>> {
        self.slots.get(index)
    }

    /// Returns a reference to the current (newest) slot.
    #[cfg(test)]
    #[inline]
    pub(crate) fn current_slot(&self) -> &Slot<T> {
        self.slots.back().unwrap()
    }

    /// Returns the total number of values recorded across all slots.
    pub fn total(&self) -> u64 {
        self.aggregate_buckets.iter().sum()
    }

    /// Calculates the value at the given percentile.
    ///
    /// Returns the minimum value of the bucket containing the percentile.
    /// For example, `percentile(0.5)` returns P50 (median), `percentile(0.99)` returns P99.
    ///
    /// Returns `0` if the histogram is empty.
    #[allow(dead_code)]
    pub fn percentile(&self, p: f64) -> u64 {
        let total = self.total();
        self.percentile_with_total(p, total)
    }

    /// Calculates the percentile given a specific total count.
    ///
    /// This is used internally when calculating multiple percentiles to avoid
    /// recalculating the total multiple times.
    fn percentile_with_total(&self, p: f64, total: u64) -> u64 {
        if total == 0 {
            return 0;
        }

        let target = (total as f64 * p).ceil().max(1.0) as u64;
        let mut cumulative = 0u64;

        for (bucket_index, &count) in self.aggregate_buckets.iter().enumerate() {
            cumulative += count;
            if cumulative >= target {
                return self.log_scale.bucket_min_value(bucket_index);
            }
        }

        0
    }

    /// Returns common percentile statistics: samples, P0.1, P1, P5, P10, P50, P90, P99, P99.9.
    pub fn percentile_stats(&self) -> PercentileStats {
        let samples = self.total();
        PercentileStats {
            samples,
            p0_1: self.percentile_with_total(0.001, samples),
            p1: self.percentile_with_total(0.01, samples),
            p5: self.percentile_with_total(0.05, samples),
            p10: self.percentile_with_total(0.10, samples),
            p50: self.percentile_with_total(0.50, samples),
            p90: self.percentile_with_total(0.90, samples),
            p99: self.percentile_with_total(0.99, samples),
            p99_9: self.percentile_with_total(0.999, samples),
        }
    }

    #[cfg(test)]
    pub(crate) fn get_bucket(&self, index: usize) -> u64 {
        self.aggregate_buckets[index]
    }

    #[cfg(test)]
    pub(crate) fn num_buckets(&self) -> usize {
        self.log_scale.num_buckets()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::histogram::LogScale3;
    use crate::base::histogram::LogScaleConfig;

    #[test]
    fn test_slot_clear() {
        let mut slot: Slot<String> = Slot::new(10);
        slot.buckets[0] = 5;
        slot.buckets[5] = 10;
        slot.data = Some("test".to_string());

        slot.clear();

        assert!(slot.buckets.iter().all(|&c| c == 0));
        assert_eq!(slot.data, None);
    }

    #[test]
    fn test_histogram_default() {
        let hist: Histogram = Histogram::default();
        assert_eq!(hist.capacity(), 1);
        assert_eq!(hist.active_slot_count(), 1);
        assert_eq!(hist.total(), 0);
    }

    #[test]
    fn test_record_and_total() {
        let mut hist: Histogram = Histogram::new();

        hist.record(1);
        hist.record(5);
        hist.record(10);
        hist.record(100);

        assert_eq!(hist.total(), 4);
        assert_eq!(hist.get_bucket(1), 1);
        assert_eq!(hist.get_bucket(5), 1);
        assert_eq!(hist.get_bucket(LogScale3::calculate_bucket_uncached(10)), 1);
        assert_eq!(hist.get_bucket(LogScale3::calculate_bucket_uncached(100)), 1);
    }

    #[test]
    fn test_record_same_bucket() {
        let mut hist: Histogram = Histogram::new();

        hist.record(8);
        hist.record(8);
        hist.record(8);

        assert_eq!(hist.total(), 3);
        assert_eq!(hist.get_bucket(8), 3);
    }

    #[test]
    fn test_u64_max_coverage() {
        let max_bucket = LogScale3::calculate_bucket_uncached(u64::MAX);
        assert_eq!(max_bucket, 251, "u64::MAX should map to bucket 251");
        assert_eq!(LogScaleConfig::<3>::BUCKETS, 252, "Should need exactly 252 buckets");

        // Verify new() creates enough buckets to record u64::MAX
        let mut hist: Histogram = Histogram::new();
        assert_eq!(hist.num_buckets(), 252);
        hist.record(u64::MAX);
        assert_eq!(hist.get_bucket(251), 1);
        assert_eq!(hist.total(), 1);
    }

    #[test]
    fn test_percentile_empty() {
        let hist: Histogram = Histogram::new();
        assert_eq!(hist.percentile(0.5), 0);
        assert_eq!(hist.percentile_stats(), PercentileStats {
            samples: 0,
            p0_1: 0,
            p1: 0,
            p5: 0,
            p10: 0,
            p50: 0,
            p90: 0,
            p99: 0,
            p99_9: 0
        });
    }

    #[test]
    fn test_percentile_single_value() {
        let mut hist: Histogram = Histogram::new();
        hist.record(10);

        assert_eq!(hist.percentile(0.0), 10);
        assert_eq!(hist.percentile(0.5), 10);
        assert_eq!(hist.percentile(0.99), 10);
        assert_eq!(hist.percentile(1.0), 10);
    }

    #[test]
    fn test_percentile_multiple_values() {
        let mut hist: Histogram = Histogram::new();

        // Record 100 values: 1-10 each recorded 10 times
        for value in 1..=10 {
            for _ in 0..10 {
                hist.record(value);
            }
        }

        assert_eq!(hist.total(), 100);

        // P50 should be around value 5-6 (bucket returns min value)
        let p50 = hist.percentile(0.5);
        assert!((4..=6).contains(&p50), "P50 = {}", p50);

        // P90 should be around value 9 (bucket 8 contains [8,9])
        let p90 = hist.percentile(0.9);
        assert!((8..=10).contains(&p90), "P90 = {}", p90);

        // P99 should be around value 10
        let p99 = hist.percentile(0.99);
        assert!((8..=10).contains(&p99), "P99 = {}", p99);
    }

    #[test]
    fn test_percentile_stats() {
        let mut hist: Histogram = Histogram::new();

        for i in 1..=100 {
            hist.record(i);
        }

        let stats = hist.percentile_stats();

        // Due to logarithmic bucketing, values are grouped
        // P50 around 50, bucket min value might be 48
        assert!(stats.p50 >= 48 && stats.p50 <= 52, "P50 = {}", stats.p50);
        // P90 around 90, bucket min value might be 80
        assert!(stats.p90 >= 80 && stats.p90 <= 92, "P90 = {}", stats.p90);
        // P99 around 99, bucket min value might be 96
        assert!(stats.p99 >= 96 && stats.p99 <= 100, "P99 = {}", stats.p99);
    }

    #[test]
    fn test_percentile_large_values() {
        let mut hist: Histogram = Histogram::new();

        // Record exponentially distributed values
        hist.record(1);
        hist.record(10);
        hist.record(100);
        hist.record(1000);
        hist.record(10000);

        assert_eq!(hist.total(), 5);

        // P50 (median) should be the 3rd value (100), but bucket returns min value (96)
        let p50 = hist.percentile(0.5);
        assert!((96..=100).contains(&p50), "P50 = {}", p50);

        // P80 should be the 4th value (1000), but bucket returns min value
        let p80 = hist.percentile(0.8);
        assert!((896..=1000).contains(&p80), "P80 = {}", p80);
    }

    // Multi-slot tests

    #[test]
    fn test_with_slots_creates_correct_capacity() {
        let hist: Histogram<u64> = Histogram::with_slots(4);
        assert_eq!(hist.capacity(), 4);
        assert_eq!(hist.active_slot_count(), 1);
    }

    #[test]
    fn test_advance_single_slot() {
        let mut hist: Histogram<u64> = Histogram::new();
        // With capacity=1, advance evicts the old slot and adds new one
        assert_eq!(hist.advance(10), 1);
        assert_eq!(hist.current_slot().data, Some(10));
    }

    #[test]
    fn test_advance_multi_slot_not_full() {
        let mut hist: Histogram<u64> = Histogram::with_slots(4);

        hist.record(100);
        assert_eq!(hist.active_slot_count(), 1);
        assert_eq!(hist.total(), 1);

        // Advance adds new slot (now 2 slots)
        assert_eq!(hist.advance(10), 2);
        hist.record(200);
        assert_eq!(hist.active_slot_count(), 2);
        assert_eq!(hist.total(), 2);
        assert_eq!(hist.current_slot().data, Some(10));

        // Advance adds new slot (now 3 slots)
        assert_eq!(hist.advance(20), 3);
        assert_eq!(hist.active_slot_count(), 3);

        // Advance adds new slot (now 4 slots = full)
        assert_eq!(hist.advance(30), 4);
        assert_eq!(hist.active_slot_count(), 4);
    }

    #[test]
    fn test_advance_evicts_oldest() {
        let mut hist: Histogram<u64> = Histogram::with_slots(4);

        hist.record(100); // initial slot
        hist.advance(10); // slot with data=10
        hist.record(200); // record to current
        hist.advance(20); // slot with data=20
        hist.advance(30); // slot with data=30, now at capacity

        assert_eq!(hist.active_slot_count(), 4);
        assert_eq!(hist.total(), 2); // 100 in slot 0, 200 in slot 1

        // Advance again - evicts oldest (slot with 100), adds new slot
        assert_eq!(hist.advance(40), 4);
        assert_eq!(hist.active_slot_count(), 4);
        assert_eq!(hist.total(), 1); // Only 200 remains (in what is now slot 0)

        // After eviction, slots shifted:
        // slot 0: was slot 1 (has 200, data=10)
        // slot 1: was slot 2 (data=20)
        // slot 2: was slot 3 (data=30)
        // slot 3: new slot (data=40)
        assert_eq!(hist.slot(0).unwrap().data, Some(10));
        assert_eq!(hist.current_slot().data, Some(40));
    }

    #[test]
    fn test_advance_capacity_stays_constant() {
        let mut hist: Histogram<u64> = Histogram::with_slots(3);
        assert_eq!(hist.capacity(), 3);

        // Fill to capacity
        hist.advance(1);
        hist.advance(2);
        assert_eq!(hist.active_slot_count(), 3);
        assert_eq!(hist.capacity(), 3);

        // Advance multiple times past capacity - capacity must not grow
        for i in 3..10 {
            hist.advance(i);
            assert_eq!(hist.capacity(), 3, "capacity grew unexpectedly at iteration {}", i);
            assert_eq!(hist.active_slot_count(), 3);
        }

        // Verify oldest slots were evicted - only last 3 data values remain
        assert_eq!(hist.slot(0).unwrap().data, Some(7));
        assert_eq!(hist.slot(1).unwrap().data, Some(8));
        assert_eq!(hist.slot(2).unwrap().data, Some(9));
    }

    #[test]
    fn test_slot_data_access() {
        let mut hist: Histogram<String> = Histogram::with_slots(3);

        // Initially 1 slot with no data
        assert_eq!(hist.slot(0).unwrap().data, None);

        // Advance adds new slot with data
        hist.advance("first".to_string());
        assert_eq!(hist.active_slot_count(), 2);
        assert_eq!(hist.current_slot().data, Some("first".to_string()));

        // Advance adds another slot with data
        hist.advance("second".to_string());
        assert_eq!(hist.active_slot_count(), 3);
        assert_eq!(hist.current_slot().data, Some("second".to_string()));
    }

    #[test]
    fn test_percentile_across_slots() {
        let mut hist: Histogram<u64> = Histogram::with_slots(4);

        // Record in initial slot
        for v in 1..=50 {
            hist.record(v);
        }

        hist.advance(1);

        // Record in new slot
        for v in 51..=100 {
            hist.record(v);
        }

        assert_eq!(hist.total(), 100);

        // P50 should be around 50
        let p50 = hist.percentile(0.5);
        assert!((48..=52).contains(&p50), "P50 = {}", p50);
    }

    #[test]
    #[should_panic(expected = "capacity must be at least 1")]
    fn test_with_slots_zero_panics() {
        let _: Histogram = Histogram::with_slots(0);
    }

    #[test]
    fn test_aggregate_buckets_consistency() {
        let mut hist: Histogram<u64> = Histogram::with_slots(3);

        // Record values in first slot
        for v in [1, 10, 100, 1000] {
            hist.record(v);
        }

        // Helper to compute manual total from slots
        let manual_total = |h: &Histogram<u64>| -> u64 {
            (0..h.active_slot_count()).flat_map(|i| h.slot(i).unwrap().buckets.iter()).sum()
        };

        assert_eq!(hist.total(), manual_total(&hist));
        assert_eq!(hist.total(), 4);

        // Advance and record more
        hist.advance(1);
        for v in [2, 20, 200] {
            hist.record(v);
        }
        assert_eq!(hist.total(), manual_total(&hist));
        assert_eq!(hist.total(), 7);

        // Fill to capacity
        hist.advance(2);
        hist.record(3);
        assert_eq!(hist.active_slot_count(), 3);
        assert_eq!(hist.total(), manual_total(&hist));
        assert_eq!(hist.total(), 8);

        // Advance past capacity - evicts first slot with [1,10,100,1000]
        hist.advance(3);
        assert_eq!(hist.active_slot_count(), 3);
        assert_eq!(hist.total(), manual_total(&hist));
        assert_eq!(hist.total(), 4); // 7 values recorded in slots 1,2 minus evicted = 3 + 1 = 4
    }
}
