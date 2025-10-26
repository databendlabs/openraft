use std::fmt;

/// Percentile statistics for a histogram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PercentileStats {
    /// 50th percentile (median)
    pub(crate) p50: u64,
    /// 90th percentile
    pub(crate) p90: u64,
    /// 99th percentile
    pub(crate) p99: u64,
}

impl fmt::Display for PercentileStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[P50: {}, P90: {}, P99: {}]", self.p50, self.p90, self.p99)
    }
}
