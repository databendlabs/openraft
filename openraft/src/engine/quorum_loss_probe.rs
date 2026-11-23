use std::time::Duration;

use crate::rt::Instant;

/// The quiet periods of a Leader whose lease has expired.
///
/// With [`Config::quorum_loss_probe_interval`] set, such a Leader alternates quiet and send
/// periods of `interval`, starting with a quiet one. In a quiet period it sends nothing to its
/// followers: no heartbeat, no log entries and no snapshot. A follower renews its lease of the
/// Leader on every message it accepts, so a single message would keep it from voting.
///
/// [`Config::quorum_loss_probe_interval`]: crate::Config::quorum_loss_probe_interval
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
pub(crate) struct QuorumLossProbe<I>
where I: Instant
{
    /// The sending time of the last RPC acknowledged by a quorum, or the Vote's last-modified
    /// time before the first acknowledgement.
    pub(crate) quorum_acked_at: I,

    pub(crate) leader_lease: Duration,

    pub(crate) interval: Duration,
}

impl<I> QuorumLossProbe<I>
where I: Instant
{
    /// Return how long the quiet period `now` falls in still lasts, or `None` if the Leader may
    /// send at `now`.
    pub(crate) fn quiet_for(&self, now: I) -> Option<Duration> {
        let (is_quiet, remaining) = self.phase(now);
        if is_quiet { Some(remaining) } else { None }
    }

    /// Return whether sending is suppressed and how long the current phase still lasts.
    pub(crate) fn phase(&self, now: I) -> (bool, Duration) {
        let age = now.saturating_duration_since(self.quorum_acked_at);
        let Some(elapsed) = age.checked_sub(self.leader_lease) else {
            return (false, self.leader_lease - age);
        };

        debug_assert!(!self.interval.is_zero());
        let interval = self.interval.as_nanos();
        let slot = elapsed.as_nanos() / interval;
        let is_quiet = slot.is_multiple_of(2);

        // The offset into the slot can exceed `u64::MAX` nanoseconds, so build it from seconds and
        // nanoseconds. It is less than `interval`, so its seconds fit in `u64`.
        let into_slot = elapsed.as_nanos() % interval;
        let into_slot_secs = (into_slot / 1_000_000_000) as u64;
        let into_slot_nanos = (into_slot % 1_000_000_000) as u32;
        let into_slot = Duration::new(into_slot_secs, into_slot_nanos);
        let remaining = self.interval.saturating_sub(into_slot);
        (is_quiet, remaining)
    }
}

#[cfg(test)]
mod quorum_loss_probe_test;
