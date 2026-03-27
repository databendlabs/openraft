use std::fmt;

use crate::base::histogram::Histogram;

/// Stage-to-stage duration histograms in microseconds.
///
/// Each histogram tracks the distribution of durations between consecutive
/// lifecycle stages across all observed segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogStageHistograms {
    pub proposed_to_received: Histogram,
    pub received_to_appended: Histogram,
    pub appended_to_persisted: Histogram,
    pub persisted_to_committed: Histogram,
    pub committed_to_applied: Histogram,
    /// End-to-end latency from proposal to apply completion.
    pub proposed_to_applied: Histogram,
}

impl LogStageHistograms {
    #[allow(dead_code)]
    pub(crate) fn new() -> Self {
        Self {
            proposed_to_received: Histogram::new(),
            received_to_appended: Histogram::new(),
            appended_to_persisted: Histogram::new(),
            persisted_to_committed: Histogram::new(),
            committed_to_applied: Histogram::new(),
            proposed_to_applied: Histogram::new(),
        }
    }
}

impl fmt::Display for LogStageHistograms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stages = [
            ("proposed→received", &self.proposed_to_received),
            ("received→appended", &self.received_to_appended),
            ("appended→persisted", &self.appended_to_persisted),
            ("persisted→committed", &self.persisted_to_committed),
            ("committed→applied", &self.committed_to_applied),
            ("proposed→applied", &self.proposed_to_applied),
        ];

        for (name, hist) in &stages {
            let stats = hist.percentile_stats();
            writeln!(
                f,
                "{:>22}: n={} p50={}us p90={}us p99={}us p99.9={}us",
                name, stats.samples, stats.p50, stats.p90, stats.p99, stats.p99_9,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_histograms_display() {
        let h = LogStageHistograms::new();
        let s = format!("{}", h);
        assert!(s.contains("proposed→received"));
        assert!(s.contains("proposed→applied"));
    }
}
