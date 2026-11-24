use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::duration_histogram::DurationHistogram;

/// What a node's router has sent since the node started, and the round trips of its heartbeats
/// since the last [`take_hb_rtt()`](Self::take_hb_rtt).
#[derive(Default)]
pub(crate) struct RouterStats {
    appends: AtomicU64,
    rpcs: AtomicU64,
    votes: AtomicU64,
    hb_failed: AtomicU64,
    hb_rtt_us: Mutex<DurationHistogram>,
}

/// The counters of [`RouterStats`] at one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RouterCounts {
    /// AppendEntries requests, with or without entries.
    pub(crate) appends: u64,

    /// The gRPC calls that carried the AppendEntries requests.
    pub(crate) rpcs: u64,

    /// Vote and PreVote requests.
    pub(crate) votes: u64,

    /// AppendEntries requests without entries that got no reply.
    pub(crate) hb_failed: u64,
}

impl RouterCounts {
    /// Return what was counted between `earlier` and `self`.
    pub(crate) fn since(&self, earlier: &RouterCounts) -> RouterCounts {
        RouterCounts {
            appends: self.appends - earlier.appends,
            rpcs: self.rpcs - earlier.rpcs,
            votes: self.votes - earlier.votes,
            hb_failed: self.hb_failed - earlier.hb_failed,
        }
    }
}

impl RouterStats {
    pub(crate) fn count_append(&self) {
        self.appends.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn count_rpc(&self) {
        self.rpcs.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn count_vote(&self) {
        self.votes.fetch_add(1, Ordering::Relaxed);
    }

    /// Record the round trip of a heartbeat that got a reply.
    pub(crate) fn heartbeat_replied(&self, rtt: Duration) {
        let micros = u64::try_from(rtt.as_micros()).unwrap_or(u64::MAX);
        self.hb_rtt_us.lock().expect("no thread panics while it holds the lock").record(micros);
    }

    pub(crate) fn heartbeat_failed(&self) {
        self.hb_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn counts(&self) -> RouterCounts {
        RouterCounts {
            appends: self.appends.load(Ordering::Relaxed),
            rpcs: self.rpcs.load(Ordering::Relaxed),
            votes: self.votes.load(Ordering::Relaxed),
            hb_failed: self.hb_failed.load(Ordering::Relaxed),
        }
    }

    /// Return the heartbeat round trips recorded so far, and start over.
    pub(crate) fn take_hb_rtt(&self) -> DurationHistogram {
        let mut hb_rtt_us = self.hb_rtt_us.lock().expect("no thread panics while it holds the lock");
        std::mem::take(&mut *hb_rtt_us)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::RouterCounts;
    use super::RouterStats;

    #[test]
    fn test_counts_since_an_earlier_moment() {
        let stats = RouterStats::default();
        stats.count_append();
        let earlier = stats.counts();

        stats.count_append();
        stats.count_rpc();
        stats.count_vote();
        stats.heartbeat_failed();

        let expected = RouterCounts {
            appends: 1,
            rpcs: 1,
            votes: 1,
            hb_failed: 1,
        };
        assert_eq!(expected, stats.counts().since(&earlier));
    }

    #[test]
    fn test_taking_the_round_trips_starts_over() {
        let stats = RouterStats::default();
        stats.heartbeat_replied(Duration::from_micros(250));

        assert_eq!(Some(250), stats.take_hb_rtt().max());
        assert_eq!(None, stats.take_hb_rtt().max());
    }
}
