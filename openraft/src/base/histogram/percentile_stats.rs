use std::fmt;

/// Percentile statistics for a histogram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PercentileStats {
    /// Total number of samples recorded
    pub total: u64,
    /// 1st percentile (99% of values >= this)
    pub p1: u64,
    /// 5th percentile (95% of values >= this)
    pub p5: u64,
    /// 10th percentile (90% of values >= this)
    pub p10: u64,
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
            "[total: {}, P1: {}, P5: {}, P10: {}, P50: {}, P90: {}, P99: {}]",
            self.total, self.p1, self.p5, self.p10, self.p50, self.p90, self.p99
        )
    }
}
