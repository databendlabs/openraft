use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::duration_histogram::DurationHistogram;

/// What a node's writer has written since it started, and the latencies of its writes since the
/// last [`take_latency()`](Self::take_latency).
#[derive(Default)]
pub(crate) struct WriteStats {
    /// Writes per second to each group; 0 until the writer starts.
    rate: AtomicU64,
    committed: AtomicU64,
    rejected: AtomicU64,
    latency_us: Mutex<DurationHistogram>,
}

/// The counters of [`WriteStats`] at one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WriteCounts {
    /// The writes that committed.
    pub(crate) committed: u64,

    /// The writes that failed.
    pub(crate) rejected: u64,
}

impl WriteCounts {
    /// Return what was counted between `earlier` and `self`.
    pub(crate) fn since(&self, earlier: &WriteCounts) -> WriteCounts {
        WriteCounts {
            committed: self.committed - earlier.committed,
            rejected: self.rejected - earlier.rejected,
        }
    }
}

impl WriteStats {
    pub(crate) fn set_rate(&self, rate: u64) {
        self.rate.store(rate, Ordering::Relaxed);
    }

    pub(crate) fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    /// Record a write that committed `latency` after its scheduled time.
    pub(crate) fn committed(&self, latency: Duration) {
        self.committed.fetch_add(1, Ordering::Relaxed);
        let micros = u64::try_from(latency.as_micros()).unwrap_or(u64::MAX);
        self.latency_us.lock().expect("no thread panics while it holds the lock").record(micros);
    }

    pub(crate) fn rejected(&self) {
        self.rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn counts(&self) -> WriteCounts {
        WriteCounts {
            committed: self.committed.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
        }
    }

    /// Return the latencies recorded so far, and start over.
    pub(crate) fn take_latency(&self) -> DurationHistogram {
        let mut latency_us = self.latency_us.lock().expect("no thread panics while it holds the lock");
        std::mem::take(&mut *latency_us)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::WriteCounts;
    use super::WriteStats;

    #[test]
    fn test_counts_since_an_earlier_moment() {
        let stats = WriteStats::default();
        stats.committed(Duration::from_micros(10));
        let earlier = stats.counts();

        stats.committed(Duration::from_micros(20));
        stats.rejected();

        let expected = WriteCounts {
            committed: 1,
            rejected: 1,
        };
        assert_eq!(expected, stats.counts().since(&earlier));
    }

    #[test]
    fn test_taking_the_latencies_starts_over() {
        let stats = WriteStats::default();
        stats.committed(Duration::from_micros(250));

        assert_eq!(Some(250), stats.take_latency().max());
        assert_eq!(None, stats.take_latency().max());
    }
}
