use std::time::Instant;

use super::measurement::Bounds;
use super::request_sample::Outcome;

/// Acknowledged entries of one controlled batch; unread responses remain uncertain.
pub(super) struct WriteProgress {
    epoch: Instant,
    bounds: Bounds,
    pub(super) entries: u64,
    pub(super) completed: u64,
    pub(super) failed: u64,
    pub(super) window_completed: u64,
    pub(super) window_failed: u64,
}

impl WriteProgress {
    pub(super) fn new(epoch: Instant, bounds: Bounds, entries: u64) -> Self {
        Self {
            epoch,
            bounds,
            entries,
            completed: 0,
            failed: 0,
            window_completed: 0,
            window_failed: 0,
        }
    }

    pub(super) fn acknowledge(&mut self, successful: bool) {
        if self.bounds.end.is_none() {
            self.record_acknowledgement(successful, true);
            return;
        }
        self.acknowledge_at(successful, Instant::now());
    }

    pub(super) fn acknowledge_at(&mut self, successful: bool, at: Instant) {
        let nanos = at.saturating_duration_since(self.epoch).as_nanos() as u64;
        let in_window = self.bounds.contains(nanos);
        self.record_acknowledgement(successful, in_window);
    }

    fn record_acknowledgement(&mut self, successful: bool, in_window: bool) {
        assert!(
            self.completed + self.failed < self.entries,
            "batch responses match its entries"
        );
        if successful {
            self.completed += 1;
            self.window_completed += u64::from(in_window);
        } else {
            self.failed += 1;
            self.window_failed += u64::from(in_window);
        }
    }

    pub(super) fn outcomes(&self, outcome: &Outcome) -> [u64; 6] {
        let mut counts = [0; 6];
        counts[0] = self.completed;
        counts[1] = self.failed;
        let remaining = self.entries - self.completed - self.failed;
        if *outcome == Outcome::Completed {
            assert_eq!(0, remaining, "a complete batch acknowledges every entry");
        }
        let uncertain = if matches!(outcome, Outcome::Failed(_)) {
            4
        } else {
            outcome.index()
        };
        counts[uncertain] += remaining;
        counts
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::Bounds;
    use super::Outcome;
    use super::WriteProgress;

    #[test]
    fn a_prefix_survives_timeout_and_stream_error() {
        let bounds = Bounds {
            start: Duration::ZERO,
            end: None,
            deadline: None,
        };
        let mut progress = WriteProgress::new(Instant::now(), bounds, 4);
        progress.record_acknowledgement(true, true);
        progress.record_acknowledgement(true, false);
        let actual = progress.outcomes(&Outcome::TimedOut);
        assert_eq!([2, 0, 2, 0, 0, 0], actual);
        let actual = progress.outcomes(&Outcome::Failed("stream closed".into()));
        assert_eq!([2, 0, 0, 0, 2, 0], actual);
        progress.record_acknowledgement(false, true);
        let actual = progress.outcomes(&Outcome::Failed("entry error".into()));
        assert_eq!([2, 1, 0, 0, 1, 0], actual);
        assert_eq!(1, progress.window_completed);
        assert_eq!(1, progress.window_failed);
    }
}
