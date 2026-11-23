use std::fmt;
use std::ops::Range;
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
/// Each stage uses a `RangeMap` that maps `(log_index, instant)` per batch.
/// The gap between stages reveals where latency accumulates
/// (channel queue, storage, replication, state machine apply, etc.).
///
/// Stages (in order): Proposed, Received, Submitted, Persisted, Committed, Applied.
/// `RaftCore` records each stage by name; `segments()` and the display methods read them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogStages<I>
where I: Instant
{
    begin: u64,
    inner: MultiRangeMap<u64, I, { Stage::COUNT }>,

    /// Sorted, disjoint ranges of logs after `begin` whose stages this node did not observe, such
    /// as the logs an installed snapshot covers: their segments are not yielded.
    excluded: Vec<Range<u64>>,

    /// The most ranges `excluded` holds, the stage capacity: one more forgets the history before
    /// the oldest of them.
    capacity: usize,
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
///
/// The per-stage wrappers round out the API rather than serving a current caller; only `new` is
/// used today.
#[allow(dead_code)]
impl<I> LogStages<I>
where I: Instant
{
    pub(crate) fn new(capacity: usize, begin: u64) -> Self {
        Self {
            begin,
            inner: MultiRangeMap::new(capacity),
            excluded: Vec::new(),
            capacity,
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
        // Back-fill earlier stages so the segment intersection stays non-empty.
        //
        // `segments()` only yields where every stage has a value, and on a
        // follower `Proposed` and `Received` are never recorded — those events
        // only happen on the proposing leader. Without back-fill, every
        // follower's log-stage histogram would stay empty.
        for i in (0..stage.index()).rev() {
            if self.inner.get_mut(i).end().is_some_and(|e| e >= right) {
                break;
            }
            self.record_at(i, right, value);
        }

        self.record_at(stage.index(), right, value);
    }

    /// Discard the stages of the logs from index `since` on, which a truncation removed from the
    /// log, so that the logs written at those indexes later are observed afresh.
    ///
    /// A batch that holds `since` keeps its stages for the logs before `since`. When `since` is
    /// before the retained history, the history is gone, and the observation starts again at
    /// `since`.
    pub(crate) fn truncate(&mut self, since: u64) {
        self.inner.truncate(self.begin, since);
        self.begin = self.begin.min(since);
        self.excluded.retain_mut(|range| {
            range.end = range.end.min(since);
            range.start < range.end
        });
    }

    /// Exclude the logs in `range` from the observation, such as the logs an installed snapshot
    /// covers: this node observes none of their stages, and the stages that a later record gives
    /// them would describe other logs.
    ///
    /// The stages recorded before and after `range` stay. A `range` that starts at or before the
    /// observation start moves the start to its end instead.
    pub(crate) fn exclude(&mut self, range: Range<u64>) {
        if range.start >= range.end {
            return;
        }

        let mut merged = range;
        self.excluded.retain(|r| {
            let disjoint = r.end < merged.start || merged.end < r.start;
            if !disjoint {
                merged = merged.start.min(r.start)..merged.end.max(r.end);
            }
            disjoint
        });
        let at = self.excluded.partition_point(|r| r.end < merged.start);
        self.excluded.insert(at, merged);

        // Bound the memory: forget the history before the oldest excluded range.
        while self.excluded.len() > self.capacity {
            let oldest = self.excluded.remove(0);
            self.begin = self.begin.max(oldest.end);
        }
        self.trim_excluded();
    }

    /// Drop the excluded ranges that start at or before the observation start, moving the start
    /// to the end of each: no observed history lies between the start and such a range.
    fn trim_excluded(&mut self) {
        while let Some(first) = self.excluded.first() {
            if first.start > self.begin {
                return;
            }
            self.begin = self.begin.max(first.end);
            self.excluded.remove(0);
        }
    }

    /// Record `(right, value)` into the range map at `index`, advancing
    /// `begin` if the insertion evicted the oldest entry.
    fn record_at(&mut self, index: usize, right: u64, value: I) {
        if let Some(evicted) = self.inner.get_mut(index).record(right, value) {
            self.begin = self.begin.max(evicted);
            self.trim_excluded();
        }
    }

    /// Iterate segments within the intersection range where all stages have data, except the
    /// logs excluded from the observation.
    pub fn segments(&self) -> SegmentIter<'_, u64, I, { Stage::COUNT }> {
        self.inner.segments(self.begin, &self.excluded)
    }

    /// Returns an iterator of formatted lines, one per segment.
    ///
    /// Each line shows the log index range, the proposed timestamp as a wall-clock time, the
    /// inter-batch gap, and per-stage step/cumulative durations. Converting a timestamp to a
    /// wall-clock time reads the current time of the runtime that recorded it; see
    /// [`Self::relative_display_lines`] for lines that read no clock.
    pub fn display_lines(&self) -> impl Iterator<Item = String> + '_ {
        self.segment_lines(|proposed| proposed.display().to_string())
    }

    /// Returns an iterator of formatted lines, one per segment, as [`Self::display_lines`] does,
    /// except that each proposed timestamp is shown as `T+<duration>`, its distance from the
    /// proposal of the first segment, `T`.
    ///
    /// It computes every value from the recorded instants alone and reads no clock, so it gives the
    /// same text whenever it runs, also after the runtime that recorded the instants has exited.
    pub fn relative_display_lines(&self) -> impl Iterator<Item = String> + '_ {
        let mut first: Option<I> = None;
        self.segment_lines(move |proposed| {
            let first = *first.get_or_insert(proposed);
            format!("T+{:.2?}", proposed.saturating_duration_since(first))
        })
    }

    /// Format each segment, with `proposed_at` rendering the segment's proposed timestamp.
    fn segment_lines<'a, F>(&'a self, mut proposed_at: F) -> impl Iterator<Item = String> + 'a
    where F: FnMut(I) -> String + 'a {
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
                proposed_at(proposed),
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

        for seg in self.segments() {
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
