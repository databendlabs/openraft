use std::sync::LazyLock;

use super::log_scale_config::LogScaleConfig;

/// Logarithmic scale with precomputed lookup tables.
///
/// Handles value-to-bucket mapping:
/// - Value → bucket index (with small-value cache)
/// - Bucket index → minimum value
///
/// Use the shared [`LOG_SCALE`] instance for WIDTH=3 (default configuration).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogScale<const WIDTH: usize> {
    /// Minimum value represented by each bucket index.
    bucket_min_values: Vec<u64>,
    /// Cached bucket indices for small values (0-4095).
    small_value_buckets: Vec<u8>,
}

impl<const WIDTH: usize> LogScale<WIDTH> {
    /// Creates a new LogScale for the given WIDTH configuration.
    pub fn new() -> Self {
        // Build bucket_min_values table
        let bucket_min_values: Vec<u64> =
            (0..LogScaleConfig::<WIDTH>::BUCKETS).map(Self::compute_bucket_min_value).collect();

        // Build small_value_buckets cache
        let small_value_buckets: Vec<u8> = (0..LogScaleConfig::<WIDTH>::SMALL_VALUE_CACHE_SIZE)
            .map(|v| Self::calculate_bucket_uncached(v as u64) as u8)
            .collect();

        Self {
            bucket_min_values,
            small_value_buckets,
        }
    }

    /// Returns the number of buckets.
    #[inline]
    pub fn num_buckets(&self) -> usize {
        self.bucket_min_values.len()
    }

    /// Returns the minimum value for the given bucket index.
    #[inline]
    pub fn bucket_min_value(&self, bucket: usize) -> u64 {
        self.bucket_min_values[bucket]
    }

    /// Calculates bucket index for a value, using cache for small values.
    #[inline]
    pub fn calculate_bucket(&self, value: u64) -> usize {
        if value < self.small_value_buckets.len() as u64 {
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
    ///    - Determine which group of GROUP_SIZE buckets
    ///    - Extract offset within that group using the bits after MSB
    ///    - Bucket index = base of this group + offset within group
    pub fn calculate_bucket_uncached(value: u64) -> usize {
        if value < LogScaleConfig::<WIDTH>::GROUP_SIZE as u64 {
            return value as usize;
        }
        let bits_upto_msb = (u64::BITS - value.leading_zeros()) as usize;
        let group_index = bits_upto_msb - LogScaleConfig::<WIDTH>::WIDTH;
        let offset_in_group = ((value >> group_index) & LogScaleConfig::<WIDTH>::MASK) as usize;
        LogScaleConfig::<WIDTH>::GROUP_SIZE + group_index * LogScaleConfig::<WIDTH>::GROUP_SIZE + offset_in_group
    }

    /// Computes the minimum value for a bucket index.
    fn compute_bucket_min_value(bucket: usize) -> u64 {
        if bucket < LogScaleConfig::<WIDTH>::GROUP_SIZE {
            return bucket as u64;
        }
        let group_index = (bucket - LogScaleConfig::<WIDTH>::GROUP_SIZE) / LogScaleConfig::<WIDTH>::GROUP_SIZE;
        let offset_in_group = (bucket - LogScaleConfig::<WIDTH>::GROUP_SIZE) % LogScaleConfig::<WIDTH>::GROUP_SIZE;
        ((offset_in_group | LogScaleConfig::<WIDTH>::GROUP_MSB_BIT) << group_index) as u64
    }
}

impl<const WIDTH: usize> Default for LogScale<WIDTH> {
    fn default() -> Self {
        Self::new()
    }
}

/// Default log scale with WIDTH=3 (252 buckets, ~12.5% max error).
pub type LogScale3 = LogScale<3>;

/// Shared LogScale instance for WIDTH=3 (default configuration).
pub static LOG_SCALE: LazyLock<LogScale3> = LazyLock::new(LogScale3::new);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_bucket_group_0() {
        assert_eq!(LogScale3::calculate_bucket_uncached(0), 0);
        assert_eq!(LogScale3::calculate_bucket_uncached(1), 1);
        assert_eq!(LogScale3::calculate_bucket_uncached(2), 2);
        assert_eq!(LogScale3::calculate_bucket_uncached(3), 3);
    }

    #[test]
    fn test_calculate_bucket_group_1() {
        assert_eq!(LogScale3::calculate_bucket_uncached(4), 4);
        assert_eq!(LogScale3::calculate_bucket_uncached(5), 5);
        assert_eq!(LogScale3::calculate_bucket_uncached(6), 6);
        assert_eq!(LogScale3::calculate_bucket_uncached(7), 7);
    }

    #[test]
    fn test_calculate_bucket_group_2() {
        assert_eq!(LogScale3::calculate_bucket_uncached(8), 8);
        assert_eq!(LogScale3::calculate_bucket_uncached(10), 9);
        assert_eq!(LogScale3::calculate_bucket_uncached(12), 10);
        assert_eq!(LogScale3::calculate_bucket_uncached(14), 11);
    }

    #[test]
    fn test_calculate_bucket_group_3() {
        assert_eq!(LogScale3::calculate_bucket_uncached(16), 12);
        assert_eq!(LogScale3::calculate_bucket_uncached(20), 13);
        assert_eq!(LogScale3::calculate_bucket_uncached(24), 14);
        assert_eq!(LogScale3::calculate_bucket_uncached(28), 15);
    }

    #[test]
    fn test_calculate_bucket_group_4() {
        assert_eq!(LogScale3::calculate_bucket_uncached(32), 16);
        assert_eq!(LogScale3::calculate_bucket_uncached(40), 17);
        assert_eq!(LogScale3::calculate_bucket_uncached(48), 18);
        assert_eq!(LogScale3::calculate_bucket_uncached(56), 19);
    }

    #[test]
    fn test_reasonable_bucket_ranges() {
        assert_eq!(LogScale3::calculate_bucket_uncached(1024), 36);
        assert_eq!(LogScale3::calculate_bucket_uncached(2048), 40);
        assert_eq!(LogScale3::calculate_bucket_uncached(4096), 44);

        let million = 1_048_576;
        let million_bucket = LogScale3::calculate_bucket_uncached(million);
        assert!(million_bucket < 80);

        let billion = 1_073_741_824;
        let billion_bucket = LogScale3::calculate_bucket_uncached(billion);
        assert!(billion_bucket < 120);
    }

    #[test]
    fn test_bucket_min_values_lookup_table() {
        // Group 0: [0, 1, 2, 3]
        assert_eq!(LOG_SCALE.bucket_min_value(0), 0);
        assert_eq!(LOG_SCALE.bucket_min_value(1), 1);
        assert_eq!(LOG_SCALE.bucket_min_value(2), 2);
        assert_eq!(LOG_SCALE.bucket_min_value(3), 3);

        // Group 1: [4, 5, 6, 7]
        assert_eq!(LOG_SCALE.bucket_min_value(4), 4);
        assert_eq!(LOG_SCALE.bucket_min_value(5), 5);
        assert_eq!(LOG_SCALE.bucket_min_value(6), 6);
        assert_eq!(LOG_SCALE.bucket_min_value(7), 7);

        // Group 2: [8, 10, 12, 14]
        assert_eq!(LOG_SCALE.bucket_min_value(8), 8);
        assert_eq!(LOG_SCALE.bucket_min_value(9), 10);
        assert_eq!(LOG_SCALE.bucket_min_value(10), 12);
        assert_eq!(LOG_SCALE.bucket_min_value(11), 14);

        // Group 3: [16, 20, 24, 28]
        assert_eq!(LOG_SCALE.bucket_min_value(12), 16);
        assert_eq!(LOG_SCALE.bucket_min_value(13), 20);
        assert_eq!(LOG_SCALE.bucket_min_value(14), 24);
        assert_eq!(LOG_SCALE.bucket_min_value(15), 28);
    }

    #[test]
    fn test_cached_bucket_matches_uncached() {
        // Sample values across cache range to verify cache correctness
        let test_values: Vec<usize> =
            (0..100).chain((100..1000).step_by(10)).chain((1000..4096).step_by(100)).collect();

        for v in test_values {
            let cached = LOG_SCALE.calculate_bucket(v as u64);
            let uncached = LogScale3::calculate_bucket_uncached(v as u64);
            assert_eq!(cached, uncached, "Mismatch at value {}", v);
        }
    }

    #[test]
    fn test_cached_bucket_boundary() {
        // Test at cache boundary
        let last_cached = (LogScaleConfig::<3>::SMALL_VALUE_CACHE_SIZE - 1) as u64;
        let first_uncached = LogScaleConfig::<3>::SMALL_VALUE_CACHE_SIZE as u64;

        assert_eq!(
            LOG_SCALE.calculate_bucket(last_cached),
            LogScale3::calculate_bucket_uncached(last_cached)
        );
        assert_eq!(
            LOG_SCALE.calculate_bucket(first_uncached),
            LogScale3::calculate_bucket_uncached(first_uncached)
        );
    }

    #[test]
    fn test_cached_bucket_large_values() {
        // Values beyond cache should still work correctly
        let large_values = [4096, 10000, 100000, 1_000_000, u64::MAX];
        for &v in &large_values {
            assert_eq!(
                LOG_SCALE.calculate_bucket(v),
                LogScale3::calculate_bucket_uncached(v),
                "Mismatch at value {}",
                v
            );
        }
    }
}
