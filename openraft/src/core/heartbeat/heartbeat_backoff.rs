use std::time::Duration;

use crate::RaftTypeConfig;
use crate::network::Backoff;
use crate::replication::EXHAUSTED_BACKOFF_DELAY;
use crate::type_config::alias::InstantOf;

/// Spaces out the heartbeats to a target that the network reports unreachable.
///
/// After each heartbeat that finds the target unreachable, the heartbeats created within the next
/// delay of a [`Backoff`] are skipped. A delay counts from the creation of the failed heartbeat,
/// and is at most `max_delay`.
pub(crate) struct HeartbeatBackoff<C>
where C: RaftTypeConfig
{
    /// The delays to wait after the failed heartbeats, one each, in order.
    delays: Backoff,

    /// The upper bound of each delay.
    max_delay: Duration,

    /// The heartbeats created before this instant are skipped.
    retry_at: InstantOf<C>,
}

impl<C> HeartbeatBackoff<C>
where C: RaftTypeConfig
{
    /// Starts to back off after the heartbeat created at `failed_at` found the target unreachable.
    pub(crate) fn new(delays: Backoff, max_delay: Duration, failed_at: InstantOf<C>) -> Self {
        let mut backoff = Self {
            delays,
            max_delay,
            retry_at: failed_at,
        };
        backoff.defer(failed_at);
        backoff
    }

    /// Skips the heartbeats of the next delay, after the heartbeat created at `failed_at` found the
    /// target unreachable again.
    pub(crate) fn defer(&mut self, failed_at: InstantOf<C>) {
        let delay = self.delays.next().unwrap_or(EXHAUSTED_BACKOFF_DELAY);
        self.retry_at = failed_at + delay.min(self.max_delay);
    }

    /// Returns whether to send the heartbeat created at `created_at`.
    pub(crate) fn is_due(&self, created_at: InstantOf<C>) -> bool {
        created_at >= self.retry_at
    }

    /// Returns the creation time from which heartbeats are sent again.
    pub(crate) fn retry_at(&self) -> InstantOf<C> {
        self.retry_at
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::HeartbeatBackoff;
    use crate::engine::testing::UTConfig;
    use crate::network::Backoff;
    use crate::replication::EXHAUSTED_BACKOFF_DELAY;
    use crate::type_config::TypeConfigExt;

    type C = UTConfig;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn test_each_delay_is_at_most_max_delay() {
        let failed_at = C::now();
        let delays = Backoff::new([ms(100), ms(300)].into_iter());
        let mut backoff = HeartbeatBackoff::<C>::new(delays, ms(150), failed_at);

        assert!(!backoff.is_due(failed_at + ms(99)));
        assert!(backoff.is_due(failed_at + ms(100)));

        backoff.defer(failed_at + ms(100));
        assert_eq!(failed_at + ms(250), backoff.retry_at(), "300 ms is cut to 150 ms");
    }

    #[test]
    fn test_an_exhausted_backoff_waits_the_fallback_delay() {
        let failed_at = C::now();
        let delays = Backoff::new(std::iter::empty());
        let backoff = HeartbeatBackoff::<C>::new(delays, ms(10_000), failed_at);

        assert_eq!(failed_at + EXHAUSTED_BACKOFF_DELAY, backoff.retry_at());
    }
}
