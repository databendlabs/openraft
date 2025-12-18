use super::percentile_stats::PercentileStats;

/// A histogram for tracking the distribution of u64 values using logarithmic bucketing.
///
/// This histogram provides O(1) recording and efficient percentile calculation with
/// bounded memory usage (252 buckets = ~2KB), regardless of the number of samples.
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
/// Fixed at 252 buckets * 8 bytes = 2,016 bytes per histogram, covering the entire
/// u64 range [0, 2^64-1].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    /// Count of samples in each bucket.
    ///
    /// `buckets[i]` holds the number of recorded values that fall into bucket `i`.
    buckets: Vec<u64>,

    /// Minimum value represented by each bucket (precomputed lookup table).
    ///
    /// `bucket_min_values[i]` is the smallest value that maps to bucket `i`.
    /// Used when reporting percentiles to convert bucket index back to a value.
    bucket_min_values: Vec<u64>,

    /// Precomputed bucket indices for small values (0 to SMALL_VALUE_CACHE_SIZE-1).
    ///
    /// Using u8 is sufficient since values 0-4095 map to bucket indices 0-44.
    small_value_buckets: Vec<u8>,
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

impl Histogram {
    /// The width of the bit pattern used for bucketing (most significant bits).
    ///
    /// Each bucket group uses 3 bits: 1 MSB + 2 offset bits.
    const WIDTH: usize = 3;

    /// The MSB bit pattern for bucket groups.
    ///
    /// Sets the most significant bit to 1: 1 << (WIDTH - 1) = 0b100
    const GROUP_MSB_BIT: usize = 1 << (Self::WIDTH - 1);

    /// Number of buckets per group.
    ///
    /// Each group contains GROUP_MSB_BIT buckets.
    /// For WIDTH=3: GROUP_MSB_BIT = 4 buckets per group.
    const GROUP_SIZE: usize = Self::GROUP_MSB_BIT;

    /// Mask for extracting the offset within a bucket group.
    ///
    /// Extracts the (WIDTH-1) bits after the MSB: GROUP_MSB_BIT - 1 = 0b11
    const MASK: u64 = (Self::GROUP_MSB_BIT - 1) as u64;

    /// The exact number of buckets needed to cover all u64 values with logarithmic precision.
    ///
    /// Calculated as: GROUP_SIZE * (66 - WIDTH)
    /// For WIDTH=3: 4 * (66 - 3) = 4 * 63 = 252
    /// This equals bucket_index(u64::MAX) + 1
    const BUCKETS_FOR_U64: usize = Self::GROUP_SIZE * (66 - Self::WIDTH);

    /// Cache size for small value bucket lookups.
    ///
    /// Values 0-4095 map to bucket indices 0-44, fitting in u8.
    const SMALL_VALUE_CACHE_SIZE: usize = 4096;

    /// Creates a new histogram with 252 buckets, covering all u64 values.
    ///
    /// Memory usage: 252 * 8 bytes = 2,016 bytes per histogram.
    pub(crate) fn new() -> Self {
        let mut bucket_min_values = vec![0u64; Self::BUCKETS_FOR_U64];
        #[allow(clippy::needless_range_loop)]
        for i in 0..Self::BUCKETS_FOR_U64 {
            if i < 4 {
                // Group 0: [0, 1, 2, 3]
                bucket_min_values[i] = i as u64;
            } else {
                let group_index = (i - 4) / Self::GROUP_SIZE;
                let offset_in_group = (i - 4) % Self::GROUP_SIZE;
                // Minimum value: (offset_in_group | GROUP_MSB_BIT) << group_index
                bucket_min_values[i] = ((offset_in_group | Self::GROUP_MSB_BIT) << group_index) as u64;
            }
        }

        // Precompute bucket indices for small values
        let small_value_buckets: Vec<u8> =
            (0..Self::SMALL_VALUE_CACHE_SIZE).map(|v| Self::calculate_bucket_uncached(v as u64) as u8).collect();

        Self {
            buckets: vec![0; Self::BUCKETS_FOR_U64],
            bucket_min_values,
            small_value_buckets,
        }
    }

    /// Records a value to the histogram.
    pub(crate) fn record(&mut self, value: u64) {
        let bucket_index = self.calculate_bucket(value);
        self.buckets[bucket_index] += 1;
    }

    /// Calculates the bucket index for a given value, using cache for small values.
    fn calculate_bucket(&self, value: u64) -> usize {
        if value < Self::SMALL_VALUE_CACHE_SIZE as u64 {
            return self.small_value_buckets[value as usize] as usize;
        }
        Self::calculate_bucket_uncached(value)
    }

    /// Calculates the bucket index for a given value using logarithmic bucketing.
    ///
    /// Algorithm:
    /// 1. For value < GROUP_SIZE: bucket_index = value
    /// 2. For value >= GROUP_SIZE:
    ///    - Find the position of the most significant bit (MSB)
    ///    - Determine which group of GROUP_SIZE buckets (group 0 has buckets 0-3, group 1 has 4-7,
    ///      etc.)
    ///    - Extract offset within that group using the 2 bits after MSB
    ///    - Bucket index = base of this group + offset within group
    fn calculate_bucket_uncached(value: u64) -> usize {
        if value < Self::GROUP_SIZE as u64 {
            return value as usize;
        }

        let bits_upto_msb = (u64::BITS - value.leading_zeros()) as usize;
        let group_index = bits_upto_msb - Self::WIDTH;
        let offset_in_group = ((value >> group_index) & Self::MASK) as usize;

        let buckets_before_this_group = Self::GROUP_SIZE + group_index * Self::GROUP_SIZE;
        buckets_before_this_group + offset_in_group
    }

    /// Returns the total number of values recorded.
    pub fn total(&self) -> u64 {
        self.buckets.iter().sum()
    }

    /// Calculates the value at the given percentile.
    ///
    /// Returns the minimum value of the bucket containing the percentile.
    /// For example, `percentile(0.5)` returns P50 (median), `percentile(0.99)` returns P99.
    ///
    /// Returns `0` if the histogram is empty.
    #[allow(dead_code)]
    pub(crate) fn percentile(&self, p: f64) -> u64 {
        let total = self.total();
        self.percentile_with_total(p, total)
    }

    /// Calculates the percentile given a specific total count.
    ///
    /// This is used internally when calculating multiple percentiles to avoid
    /// recalculating the total multiple times.
    #[allow(dead_code)]
    fn percentile_with_total(&self, p: f64, total: u64) -> u64 {
        let target = (total as f64 * p).ceil().max(1.0) as u64;
        let mut cumulative = 0u64;

        for (bucket_index, &count) in self.buckets.iter().enumerate() {
            cumulative += count;
            if cumulative >= target {
                return self.bucket_min_values[bucket_index];
            }
        }

        0
    }

    /// Returns common percentile statistics: total, P1, P5, P10, P50, P90, P99.
    #[allow(dead_code)]
    pub fn percentile_stats(&self) -> PercentileStats {
        let total = self.total();
        PercentileStats {
            total,
            p1: self.percentile_with_total(0.01, total),
            p5: self.percentile_with_total(0.05, total),
            p10: self.percentile_with_total(0.10, total),
            p50: self.percentile_with_total(0.50, total),
            p90: self.percentile_with_total(0.90, total),
            p99: self.percentile_with_total(0.99, total),
        }
    }

    #[cfg(test)]
    pub(crate) fn get_bucket(&self, index: usize) -> u64 {
        self.buckets.get(index).copied().unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn num_buckets(&self) -> usize {
        self.buckets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_bucket_group_0() {
        assert_eq!(Histogram::calculate_bucket_uncached(0), 0);
        assert_eq!(Histogram::calculate_bucket_uncached(1), 1);
        assert_eq!(Histogram::calculate_bucket_uncached(2), 2);
        assert_eq!(Histogram::calculate_bucket_uncached(3), 3);
    }

    #[test]
    fn test_calculate_bucket_group_1() {
        assert_eq!(Histogram::calculate_bucket_uncached(4), 4);
        assert_eq!(Histogram::calculate_bucket_uncached(5), 5);
        assert_eq!(Histogram::calculate_bucket_uncached(6), 6);
        assert_eq!(Histogram::calculate_bucket_uncached(7), 7);
    }

    #[test]
    fn test_calculate_bucket_group_2() {
        assert_eq!(Histogram::calculate_bucket_uncached(8), 8);
        assert_eq!(Histogram::calculate_bucket_uncached(10), 9);
        assert_eq!(Histogram::calculate_bucket_uncached(12), 10);
        assert_eq!(Histogram::calculate_bucket_uncached(14), 11);
    }

    #[test]
    fn test_calculate_bucket_group_3() {
        assert_eq!(Histogram::calculate_bucket_uncached(16), 12);
        assert_eq!(Histogram::calculate_bucket_uncached(20), 13);
        assert_eq!(Histogram::calculate_bucket_uncached(24), 14);
        assert_eq!(Histogram::calculate_bucket_uncached(28), 15);
    }

    #[test]
    fn test_calculate_bucket_group_4() {
        assert_eq!(Histogram::calculate_bucket_uncached(32), 16);
        assert_eq!(Histogram::calculate_bucket_uncached(40), 17);
        assert_eq!(Histogram::calculate_bucket_uncached(48), 18);
        assert_eq!(Histogram::calculate_bucket_uncached(56), 19);
    }

    #[test]
    fn test_record_and_total() {
        let mut hist = Histogram::new();

        hist.record(1);
        hist.record(5);
        hist.record(10);
        hist.record(100);

        assert_eq!(hist.total(), 4);
        assert_eq!(hist.get_bucket(1), 1);
        assert_eq!(hist.get_bucket(5), 1);
        assert_eq!(hist.get_bucket(Histogram::calculate_bucket_uncached(10)), 1);
        assert_eq!(hist.get_bucket(Histogram::calculate_bucket_uncached(100)), 1);
    }

    #[test]
    fn test_record_same_bucket() {
        let mut hist = Histogram::new();

        hist.record(8);
        hist.record(8);
        hist.record(8);

        assert_eq!(hist.total(), 3);
        assert_eq!(hist.get_bucket(8), 3);
    }

    #[test]
    fn test_u64_max_coverage() {
        let max_bucket = Histogram::calculate_bucket_uncached(u64::MAX);
        assert_eq!(max_bucket, 251, "u64::MAX should map to bucket 251");
        assert_eq!(Histogram::BUCKETS_FOR_U64, 252, "Should need exactly 252 buckets");

        // Verify new() creates enough buckets to record u64::MAX
        let mut hist = Histogram::new();
        assert_eq!(hist.num_buckets(), 252);
        hist.record(u64::MAX);
        assert_eq!(hist.get_bucket(251), 1);
        assert_eq!(hist.total(), 1);
    }

    #[test]
    fn test_reasonable_bucket_ranges() {
        assert_eq!(Histogram::calculate_bucket_uncached(1024), 36);
        assert_eq!(Histogram::calculate_bucket_uncached(2048), 40);
        assert_eq!(Histogram::calculate_bucket_uncached(4096), 44);

        let million = 1_048_576;
        let million_bucket = Histogram::calculate_bucket_uncached(million);
        assert!(million_bucket < 80);

        let billion = 1_073_741_824;
        let billion_bucket = Histogram::calculate_bucket_uncached(billion);
        assert!(billion_bucket < 120);
    }

    #[test]
    fn test_bucket_min_values_lookup_table() {
        let hist = Histogram::new();

        // Group 0: [0, 1, 2, 3]
        assert_eq!(hist.bucket_min_values[0], 0);
        assert_eq!(hist.bucket_min_values[1], 1);
        assert_eq!(hist.bucket_min_values[2], 2);
        assert_eq!(hist.bucket_min_values[3], 3);

        // Group 1: [4, 5, 6, 7]
        assert_eq!(hist.bucket_min_values[4], 4);
        assert_eq!(hist.bucket_min_values[5], 5);
        assert_eq!(hist.bucket_min_values[6], 6);
        assert_eq!(hist.bucket_min_values[7], 7);

        // Group 2: [8, 10, 12, 14]
        assert_eq!(hist.bucket_min_values[8], 8);
        assert_eq!(hist.bucket_min_values[9], 10);
        assert_eq!(hist.bucket_min_values[10], 12);
        assert_eq!(hist.bucket_min_values[11], 14);

        // Group 3: [16, 20, 24, 28]
        assert_eq!(hist.bucket_min_values[12], 16);
        assert_eq!(hist.bucket_min_values[13], 20);
        assert_eq!(hist.bucket_min_values[14], 24);
        assert_eq!(hist.bucket_min_values[15], 28);
    }

    #[test]
    fn test_percentile_empty() {
        let hist = Histogram::new();
        assert_eq!(hist.percentile(0.5), 0);
        assert_eq!(hist.percentile_stats(), PercentileStats {
            total: 0,
            p1: 0,
            p5: 0,
            p10: 0,
            p50: 0,
            p90: 0,
            p99: 0
        });
    }

    #[test]
    fn test_percentile_single_value() {
        let mut hist = Histogram::new();
        hist.record(10);

        assert_eq!(hist.percentile(0.0), 10);
        assert_eq!(hist.percentile(0.5), 10);
        assert_eq!(hist.percentile(0.99), 10);
        assert_eq!(hist.percentile(1.0), 10);
    }

    #[test]
    fn test_percentile_multiple_values() {
        let mut hist = Histogram::new();

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
        let mut hist = Histogram::new();

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
        let mut hist = Histogram::new();

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

    #[test]
    fn test_cached_bucket_matches_uncached() {
        let hist = Histogram::new();

        // Sample values across cache range to verify cache correctness
        let test_values: Vec<usize> =
            (0..100).chain((100..1000).step_by(10)).chain((1000..4096).step_by(100)).collect();

        for v in test_values {
            let cached = hist.calculate_bucket(v as u64);
            let uncached = Histogram::calculate_bucket_uncached(v as u64);
            assert_eq!(cached, uncached, "Mismatch at value {}", v);
        }
    }

    #[test]
    fn test_cached_bucket_boundary() {
        let hist = Histogram::new();

        // Test at cache boundary
        let last_cached = (Histogram::SMALL_VALUE_CACHE_SIZE - 1) as u64;
        let first_uncached = Histogram::SMALL_VALUE_CACHE_SIZE as u64;

        assert_eq!(
            hist.calculate_bucket(last_cached),
            Histogram::calculate_bucket_uncached(last_cached)
        );
        assert_eq!(
            hist.calculate_bucket(first_uncached),
            Histogram::calculate_bucket_uncached(first_uncached)
        );
    }

    #[test]
    fn test_cached_bucket_large_values() {
        let hist = Histogram::new();

        // Values beyond cache should still work correctly
        let large_values = [4096, 10000, 100000, 1_000_000, u64::MAX];
        for &v in &large_values {
            assert_eq!(
                hist.calculate_bucket(v),
                Histogram::calculate_bucket_uncached(v),
                "Mismatch at value {}",
                v
            );
        }
    }
}
