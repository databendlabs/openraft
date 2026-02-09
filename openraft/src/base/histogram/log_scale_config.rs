/// Configuration for logarithmic bucket boundaries with const generic `WIDTH`.
///
/// The `WIDTH` parameter determines bucket granularity:
/// - WIDTH=3: 4 buckets per group, 252 total buckets, ~12.5% max error
/// - WIDTH=4: 8 buckets per group, 504 total buckets, ~6.25% max error
///
/// All derived constants are computed at compile time from `WIDTH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogScaleConfig<const WIDTH: usize>;

impl<const WIDTH: usize> LogScaleConfig<WIDTH> {
    /// The width of the bit pattern used for bucketing (most significant bits).
    ///
    /// Each bucket group uses WIDTH bits: 1 MSB + (WIDTH-1) offset bits.
    pub const WIDTH: usize = WIDTH;

    /// The MSB bit pattern for bucket groups.
    ///
    /// Sets the most significant bit to 1: `1 << (WIDTH - 1) = 0b100` for WIDTH=3.
    pub const GROUP_MSB_BIT: usize = 1 << (WIDTH - 1);

    /// Number of buckets per group.
    ///
    /// Each group contains GROUP_MSB_BIT buckets.
    /// For WIDTH=3: GROUP_MSB_BIT = 4 buckets per group.
    pub const GROUP_SIZE: usize = Self::GROUP_MSB_BIT;

    /// Mask for extracting the offset within a bucket group.
    ///
    /// Extracts the (WIDTH-1) bits after the MSB: `GROUP_MSB_BIT - 1 = 0b11` for WIDTH=3.
    pub const MASK: u64 = (Self::GROUP_MSB_BIT - 1) as u64;

    /// The exact number of buckets needed to cover all u64 values with logarithmic precision.
    ///
    /// Calculated as: `GROUP_SIZE * (66 - WIDTH)`
    /// For WIDTH=3: 4 * (66 - 3) = 4 * 63 = 252
    /// This equals `bucket_index(u64::MAX) + 1`.
    pub const BUCKETS: usize = Self::GROUP_SIZE * (66 - WIDTH);

    /// Cache size for small value bucket lookups.
    ///
    /// Values 0-4095 map to bucket indices 0-44, fitting in u8.
    pub const SMALL_VALUE_CACHE_SIZE: usize = 4096;
}

/// Default log scale configuration with WIDTH=3 (252 buckets, ~12.5% max error).
#[allow(dead_code)]
pub type DefaultLogScaleConfig = LogScaleConfig<3>;
