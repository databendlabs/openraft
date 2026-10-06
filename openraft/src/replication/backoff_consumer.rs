//! Consumer handle for the shared [`Backoff`] owned by
//! [`BackoffState`](crate::replication::backoff_state::BackoffState).
//!
//! The request stream samples a delay before each AppendEntries request and clears
//! the active iterator on reset. `BackoffState` tracks the error rank and enables
//! new iterators after RPC errors.

use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use rt::OptionalSend;
use rt::WatchReceiver;

use crate::RaftTypeConfig;
use crate::async_runtime::Select3;
use crate::network::Backoff;
use crate::replication::EXHAUSTED_BACKOFF_DELAY;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::WatchReceiverOf;

/// Handle for sampling delays and clearing the active backoff on reset.
///
/// Shares the iterator with
/// [`BackoffState`](crate::replication::backoff_state::BackoffState), which controls when a new
/// iterator is enabled. Constructed by
/// [`BackoffState::consumer`](crate::replication::backoff_state::BackoffState::consumer).
#[derive(Clone)]
pub(crate) struct BackoffConsumer<C>
where C: RaftTypeConfig
{
    pub(crate) inner: Arc<Mutex<Option<Backoff>>>,

    /// Backoff is reset and the sleep should be aborted.
    pub(crate) reset_rx: WatchReceiverOf<C, ()>,
}

impl<C> BackoffConsumer<C>
where C: RaftTypeConfig
{
    /// Returns the next delay to wait before emitting the next request, or `None`
    /// if backoff is not currently enabled. Advances the iterator when enabled.
    pub(crate) fn next_delay(&self) -> Option<Duration> {
        let mut guard = self.inner.lock().unwrap();
        let backoff = guard.as_mut()?;
        Some(backoff.next().unwrap_or(EXHAUSTED_BACKOFF_DELAY))
    }

    /// Waits for the next backoff delay if backoff is enabled, or returns immediately.
    ///
    /// The wait ends early when `cancel` resolves or a reset arrives. A reset also drops the
    /// iterator, so later requests are not delayed until `BackoffState` enables a new one.
    pub(crate) async fn backoff_if_enabled<Fu, T>(&mut self, cancel: Fu)
    where
        Fu: Future<Output = T> + OptionalSend,
        T: fmt::Debug,
    {
        let Some(sleep_duration) = self.next_delay() else {
            return;
        };

        let sleep = C::sleep(sleep_duration);
        let reset = self.reset_rx.changed();

        tracing::debug!("backoff timeout: {:?}", sleep_duration);

        let selected = C::select3(cancel, reset, sleep).await;
        match selected {
            Select3::Third(_) => {
                tracing::debug!("backoff timeout");
            }
            Select3::First(cancel_res) => {
                tracing::info!(
                    "Replication Stream is canceled, res: {:?}, when:(backoff_if_enabled:wait-for-changed)",
                    cancel_res
                );
            }
            Select3::Second(reset_res) => {
                tracing::info!(
                    "Backoff is reset, res: {:?}, when:(backoff_if_enabled:wait-for-changed)",
                    reset_res
                );
                // once reset is received, clear the backoff state.
                self.inner.lock().unwrap().take();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::FutureExt;
    use rt::WatchSender;

    use crate::engine::testing::UTConfig;
    use crate::network::Backoff;
    use crate::replication::backoff_state::BackoffState;
    use crate::type_config::TypeConfigExt;

    fn constant_200ms_backoff() -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(200)))
    }

    /// The consumer yields a delay only while backoff is enabled on the owning
    /// `BackoffState`, and returns `None` once the state clears it.
    #[test]
    fn samples_delay_only_when_enabled() {
        let (_tx, rx) = UTConfig::<()>::watch_channel(());
        let mut state = BackoffState::new();
        let consumer = state.consumer::<UTConfig>(rx);

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

    /// A reset during an active wait stops the delay and clears the iterator.
    #[test]
    fn reset_interrupts_active_backoff() {
        UTConfig::<()>::run(async {
            let (tx, rx) = UTConfig::<()>::watch_channel(());
            let mut state = BackoffState::new();
            let mut consumer = state.consumer::<UTConfig>(rx);

            state.on_error(100);
            state.reconcile(constant_200ms_backoff);

            let wait = consumer.backoff_if_enabled(std::future::pending::<()>());
            let mut wait = Box::pin(wait);
            let pending = wait.as_mut().now_or_never();
            assert!(pending.is_none(), "backoff should wait before reset");

            tx.send(()).unwrap();
            let result = UTConfig::<()>::timeout(Duration::from_millis(100), wait).await;
            assert!(result.is_ok(), "reset should interrupt the 200ms wait");

            let next_delay = consumer.next_delay();
            assert_eq!(next_delay, None);
        });
    }
}
