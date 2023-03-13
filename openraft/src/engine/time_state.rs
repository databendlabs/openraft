use std::time::Duration;

use tokio::time::Instant;

#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct Config {
    /// The time interval after which the next election will be initiated once the current lease has
    /// expired.
    pub(crate) election_timeout: Duration,

    /// The duration of an active leader's lease.
    ///
    /// When a follower or learner perceives an active leader, such as by receiving an AppendEntries
    /// message, it should not grant another candidate to become the leader during this period.
    pub(crate) leader_lease: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            election_timeout: Duration::from_millis(150),
            leader_lease: Duration::from_millis(150),
        }
    }
}

/// Wall clock time related state that track current wall clock time, leader lease, timeout etc.
#[derive(Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct TimeState {
    /// Cached current time.
    now: Instant,
}

impl Default for TimeState {
    fn default() -> Self {
        let now = Instant::now();
        Self { now }
    }
}

impl TimeState {
    pub(crate) fn new(now: Instant) -> Self {
        Self { now }
    }

    pub(crate) fn now(&self) -> &Instant {
        &self.now
    }

    pub(crate) fn update_now(&mut self, now: Instant) {
        tracing::debug!("update_now: {:?}", now);
        debug_assert!(now >= self.now, "monotonic time expects {:?} >= {:?}", now, self.now);

        self.now = now;
    }
}
