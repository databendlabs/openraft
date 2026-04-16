//! Read-only consumer handle for the shared [`Backoff`] owned by
//! [`BackoffState`](crate::replication::backoff_state::BackoffState).
//!
//! The request-stream generator in
//! [`StreamState`](crate::replication::stream_state::StreamState) samples the next
//! delay before emitting each AppendEntries request. It does not — and must not —
//! enable, disable, or replace the backoff itself; only `BackoffState` does that,
//! maintaining the invariants described in
//! [issue #1723](https://github.com/databendlabs/openraft/issues/1723).

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use crate::network::Backoff;

/// Fallback delay used when the [`Backoff`] iterator is exhausted.
const EXHAUSTED_BACKOFF_DELAY: Duration = Duration::from_millis(500);

/// Read-only handle to the backoff shared with the request-stream generator.
///
/// Exposes exactly one operation — [`next_delay`](Self::next_delay) — so the consumer
/// cannot enable, disable, or replace the backoff. Constructed only by
/// [`BackoffState::consumer`](crate::replication::backoff_state::BackoffState::consumer).
#[derive(Clone)]
pub(crate) struct BackoffConsumer {
    pub(crate) inner: Arc<Mutex<Option<Backoff>>>,
}

impl BackoffConsumer {
    /// Returns the next delay to wait before emitting the next request, or `None`
    /// if backoff is not currently enabled. Advances the iterator when enabled.
    pub(crate) fn next_delay(&self) -> Option<Duration> {
        let mut guard = self.inner.lock().unwrap();
        let backoff = guard.as_mut()?;
        Some(backoff.next().unwrap_or(EXHAUSTED_BACKOFF_DELAY))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::network::Backoff;
    use crate::replication::backoff_state::BackoffState;

    fn constant_200ms_backoff() -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(200)))
    }

    /// The consumer yields a delay only while backoff is enabled on the owning
    /// `BackoffState`, and returns `None` once the state clears it.
    #[test]
    fn samples_delay_only_when_enabled() {
        let mut state = BackoffState::new();
        let consumer = state.consumer();

        assert_eq!(consumer.next_delay(), None, "disabled: no delay");

        state.on_error(100);
        state.reconcile(constant_200ms_backoff);

        assert_eq!(
            consumer.next_delay(),
            Some(Duration::from_millis(200)),
            "enabled: yields configured delay"
        );

        state.on_success();

        assert_eq!(consumer.next_delay(), None, "after success: no delay");
    }
}
