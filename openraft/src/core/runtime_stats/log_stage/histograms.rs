use std::fmt;

use base2histogram::AsciiChart;
use base2histogram::Histogram;
use base2histogram::PercentileStats;

/// Stage-to-stage duration histograms in microseconds.
///
/// Each histogram tracks the distribution of durations between consecutive
/// lifecycle stages across all observed segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogStageHistograms {
    pub proposed_to_received: Histogram,
    pub received_to_submitted: Histogram,
    pub submitted_to_persisted: Histogram,
    pub persisted_to_committed: Histogram,
    pub committed_to_applied: Histogram,
    /// End-to-end latency from proposal to apply completion.
    pub proposed_to_applied: Histogram,
}

impl LogStageHistograms {
    pub(crate) fn new() -> Self {
        Self {
            proposed_to_received: Histogram::new(),
            received_to_submitted: Histogram::new(),
            submitted_to_persisted: Histogram::new(),
            persisted_to_committed: Histogram::new(),
            committed_to_applied: Histogram::new(),
            proposed_to_applied: Histogram::new(),
        }
    }
}

impl LogStageHistograms {
    pub(crate) fn percentile_stats_array(&self) -> [(&'static str, PercentileStats); 6] {
        [
            ("1:proposed→received", self.proposed_to_received.percentile_stats()),
            ("2:received→submitted", self.received_to_submitted.percentile_stats()),
            ("3:submitted→persisted", self.submitted_to_persisted.percentile_stats()),
            ("4:persisted→committed", self.persisted_to_committed.percentile_stats()),
            ("5:committed→applied", self.committed_to_applied.percentile_stats()),
            ("1~5:proposed→applied", self.proposed_to_applied.percentile_stats()),
        ]
    }

    /// Build a stacked ASCII chart of all stage-to-stage latency histograms.
    #[allow(dead_code)]
    pub(crate) fn ascii_chart(&self) -> AsciiChart {
        AsciiChart::new()
            .add("1:proposed→received", self.proposed_to_received.clone())
            .add("2:received→submitted", self.received_to_submitted.clone())
            .add("3:submitted→persisted", self.submitted_to_persisted.clone())
            .add("4:persisted→committed", self.persisted_to_committed.clone())
            .add("5:committed→applied", self.committed_to_applied.clone())
    }
}

impl fmt::Display for LogStageHistograms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stages = [
            ("proposed→received", &self.proposed_to_received),
            ("received→submitted", &self.received_to_submitted),
            ("submitted→persisted", &self.submitted_to_persisted),
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
