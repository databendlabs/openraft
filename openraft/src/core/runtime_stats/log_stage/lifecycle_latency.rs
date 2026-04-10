use std::fmt;
use std::time::Duration;

use super::LogStageHistograms;
use crate::Instant;
use crate::base::multi_range_map::MultiRangeMap;
use crate::base::multi_range_map::SegmentIter;
#[cfg(doc)]
use crate::base::range_map::RangeMap;
use crate::core::stage::Stage;
use crate::display_ext::DisplayInstantExt;

/// Tracks timestamps at 6 lifecycle stages of log entries.
///
/// Each stage uses a [`RangeMap`] that maps `(log_index, instant)` per batch.
/// The gap between stages reveals where latency accumulates
/// (channel queue, storage, replication, state machine apply, etc.).
///
/// Stages (in order): Proposed, Received, Submitted, Persisted, Committed, Applied.
/// Access by name via [`Stage`] variants or the convenience methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogStages<I>
where I: Instant
{
    begin: u64,
    inner: MultiRangeMap<u64, I, { Stage::COUNT }>,
}

impl<I> fmt::Display for LogStages<I>
where I: Instant
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for line in self.display_lines() {
            if !first {
                writeln!(f)?;
            }
            write!(f, "{}", line)?;
            first = false;
        }
        Ok(())
    }
}

/// Stage-specific convenience methods for [`LogStages`].
#[allow(dead_code)]
impl<I> LogStages<I>
where I: Instant
{
    pub(crate) fn new(capacity: usize, begin: u64) -> Self {
        Self {
            begin,
            inner: MultiRangeMap::new(capacity),
        }
    }

    pub(crate) fn proposed(&mut self, right: u64, value: I) {
        self.record_stage(Stage::Proposed, right, value);
    }

    pub(crate) fn received(&mut self, right: u64, value: I) {
        self.record_stage(Stage::Received, right, value);
    }

    pub(crate) fn submitted(&mut self, right: u64, value: I) {
        self.record_stage(Stage::Submitted, right, value);
    }

    pub(crate) fn persisted(&mut self, right: u64, value: I) {
        self.record_stage(Stage::Persisted, right, value);
    }

    pub(crate) fn committed(&mut self, right: u64, value: I) {
        self.record_stage(Stage::Committed, right, value);
    }

    pub(crate) fn applied(&mut self, right: u64, value: I) {
        self.record_stage(Stage::Applied, right, value);
    }

    pub(crate) fn record_stage(&mut self, stage: Stage, right: u64, value: I) {
        let range_map = self.inner.get_mut(stage.index());

        // Defensive check: in low-latency environments, the same stage for the same
        // log index may be recorded multiple times in quick succession.
        // Skip recording if there's no new progress (right boundary not increasing).
        if let Some(prev_end) = range_map.end() {
            if right <= prev_end {
                tracing::debug!(
                    "LogStages::record_stage: skipping non-increasing boundary, stage={:?}, right={}, prev_end={}",
                    stage,
                    right,
                    prev_end
                );
                return;
            }
        }

        if let Some(evicted) = range_map.record(right, value) {
            self.begin = self.begin.max(evicted);
        }
    }

    /// Iterate segments within the intersection range where all stages have data.
    pub fn segments(&self) -> SegmentIter<'_, u64, I, { Stage::COUNT }> {
        self.inner.segments(self.begin)
    }

    /// Returns an iterator of formatted lines, one per segment.
    ///
    /// Each line shows the log index range, the proposed timestamp, the
    /// inter-batch gap, and per-stage step/cumulative durations.
    pub fn display_lines(&self) -> impl Iterator<Item = String> + '_ {
        use std::fmt::Write;

        let mut prev_proposed: Option<I> = None;

        self.segments().map(move |seg| {
            let proposed = seg.values[Stage::Proposed.index()];
            let delta = prev_proposed.map(|p| proposed.saturating_duration_since(p)).unwrap_or_default();

            let mut line = String::new();
            write!(
                line,
                "[{},{}): {} +{:.2?}; ",
                seg.range.start,
                seg.range.end,
                proposed.display(),
                delta
            )
            .unwrap();

            let mut prev = proposed;
            let mut cumulative = Duration::default();
            for (j, (name, at)) in [
                ("proposed", seg.values[Stage::Proposed.index()]),
                ("received", seg.values[Stage::Received.index()]),
                ("submitted", seg.values[Stage::Submitted.index()]),
                ("persisted", seg.values[Stage::Persisted.index()]),
                ("committed", seg.values[Stage::Committed.index()]),
                ("applied", seg.values[Stage::Applied.index()]),
            ]
            .into_iter()
            .enumerate()
            {
                if j > 0 {
                    write!(line, ", ").unwrap();
                }
                let step = at.saturating_duration_since(prev);
                cumulative += step;
                write!(line, "{} +{:.2?} ({:.2?})", name, step, cumulative).unwrap();
                prev = at;
            }

            prev_proposed = Some(proposed);
            line
        })
    }

    /// Compute stage-to-stage duration histograms from all segments.
    pub fn compute_histograms(&self) -> LogStageHistograms {
        use Stage::*;

        let mut h = LogStageHistograms::new();

        for seg in self.inner.segments(self.begin) {
            let n = seg.range.end - seg.range.start;
            let v = &seg.values;
            h.proposed_to_received.record_n(
                v[Received.index()].saturating_duration_since(v[Proposed.index()]).as_micros() as u64,
                n,
            );
            h.received_to_submitted.record_n(
                v[Submitted.index()].saturating_duration_since(v[Received.index()]).as_micros() as u64,
                n,
            );
            h.submitted_to_persisted.record_n(
                v[Persisted.index()].saturating_duration_since(v[Submitted.index()]).as_micros() as u64,
                n,
            );
            h.persisted_to_committed.record_n(
                v[Committed.index()].saturating_duration_since(v[Persisted.index()]).as_micros() as u64,
                n,
            );
            h.committed_to_applied.record_n(
                v[Applied.index()].saturating_duration_since(v[Committed.index()]).as_micros() as u64,
                n,
            );
            h.proposed_to_applied.record_n(
                v[Applied.index()].saturating_duration_since(v[Proposed.index()]).as_micros() as u64,
                n,
            );
        }

        h
    }
}
