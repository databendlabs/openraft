use std::fmt;

/// Percentile statistics for a histogram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PercentileStats {
    /// Total number of samples recorded
    pub total: u64,
    /// 50th percentile (median)
    pub p50: u64,
    /// 90th percentile
    pub p90: u64,
    /// 99th percentile
    pub p99: u64,
}

impl fmt::Display for PercentileStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[total: {}, P50: {}, P90: {}, P99: {}]",
            self.total, self.p50, self.p90, self.p99
        )
    }
}
