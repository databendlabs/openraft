use base2histogram::PercentileStats;

/// A histogram that records nothing and allocates nothing.
///
/// Without the `runtime-stats` feature no API returns [`RuntimeStats`], so nothing can read its
/// histograms, and they use this stand-in. `base2histogram::Histogram::new()` allocates its
/// buckets, about 2 KiB, and the first one in a process also builds base2histogram's scale
/// tables, about 25.6 MiB.
///
/// It reports what an empty `base2histogram::Histogram` reports.
///
/// [`RuntimeStats`]: crate::core::runtime_stats::RuntimeStats
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NoopHistogram;

impl NoopHistogram {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn record(&mut self, _value: u64) {}

    pub(crate) fn record_n(&mut self, _value: u64, _count: u64) {}

    pub(crate) fn percentile_stats(&self) -> PercentileStats {
        PercentileStats {
            samples: 0,
            p0_1: 0,
            p1: 0,
            p5: 0,
            p10: 0,
            p50: 0,
            p90: 0,
            p99: 0,
            p99_9: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NoopHistogram;
    use crate::core::runtime_stats::log_stage::LogStageHistograms;

    #[test]
    fn test_percentile_stats_match_an_empty_histogram() {
        let empty = base2histogram::Histogram::<()>::new();

        let mut noop = NoopHistogram::new();
        noop.record(5);
        noop.record_n(7, 3);

        assert_eq!(empty.percentile_stats(), noop.percentile_stats());
    }

    #[test]
    fn test_log_stage_histograms_are_zero_sized() {
        assert_eq!(size_of::<LogStageHistograms>(), 0);
    }
}
