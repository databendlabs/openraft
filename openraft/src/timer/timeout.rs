use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use futures::future::select;
use futures::future::Either;
use tokio::sync::oneshot;
use tokio::sync::oneshot::Receiver;
use tokio::sync::oneshot::Sender;
use tokio::time::sleep_until;
use tokio::time::Instant;
use tracing::trace_span;
use tracing::Instrument;

pub(crate) trait RaftTimer {
    /// Create a new instance that will call `callback` after `timeout`.
    fn new<F: FnOnce() + Send + 'static>(callback: F, timeout: Duration) -> Self;

    /// Update the timeout to a duration since now.
    fn update_timeout(&self, timeout: Duration);
}

/// A oneshot timeout that supports deadline updating.
///
/// When timeout deadline is reached, the `callback: Fn()` is called.
/// `callback` is guaranteed to be called at most once.
///
/// The deadline can be updated to a higher value then the old deadline won't trigger the
/// `callback`.
pub(crate) struct Timeout {
    /// A guard to notify the inner-task to quit when it is dropped.
    // tx is not explicitly used.
    #[allow(dead_code)]
    tx: Sender<()>,

    /// Shared state for running the sleep-notify task.
    inner: Arc<TimeoutInner>,
}

pub(crate) struct TimeoutInner {
    /// The time when this Timeout is created.
    ///
    /// The `relative_deadline` stores timeout deadline relative to `init` in micro second.
    /// Thus a `u64` is enough for it to run for years.
    init: Instant,

    /// The micro seconds since `init` after which the callback will be triggered.
    relative_deadline: AtomicU64,
}

impl RaftTimer for Timeout {
    fn new<F: FnOnce() + Send + 'static>(callback: F, timeout: Duration) -> Self {
        let (tx, rx) = oneshot::channel();

        let inner = TimeoutInner {
            init: Instant::now(),
            relative_deadline: AtomicU64::new(timeout.as_micros() as u64),
        };

        let inner = Arc::new(inner);

        let t = Timeout {
            tx,
            inner: inner.clone(),
        };

        tokio::spawn(inner.sleep_loop(rx, callback).instrument(trace_span!("timeout-loop").or_current()));

        t
    }

    fn update_timeout(&self, timeout: Duration) {
        let since_init = Instant::now() + timeout - self.inner.init;

        let new_at = since_init.as_micros() as u64;

        self.inner.relative_deadline.fetch_max(new_at, Ordering::Relaxed);
    }
}

impl TimeoutInner {
    /// Sleep until the deadline and send callback if the deadline is not changed.
    /// Otherwise, sleep again.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn sleep_loop<F: FnOnce() + Send + 'static>(self: Arc<Self>, rx: Receiver<()>, callback: F) {
        let mut wake_up_at = None;

        let mut rx = rx;
        loop {
            let curr_deadline = self.relative_deadline.load(Ordering::Relaxed);

            if wake_up_at == Some(curr_deadline) {
                // `relative_deadline` is not updated.
                callback();
                return;
            }

            // `relative_deadline` is updated, keep sleeping.

            wake_up_at = Some(curr_deadline);

            let deadline = self.init + Duration::from_micros(curr_deadline);

            let either = select(Box::pin(sleep_until(deadline)), rx).await;
            rx = match either {
                Either::Left((_sleep_res, rx)) => {
                    tracing::debug!("sleep returned, continue to check if deadline changed");
                    rx
                }
                Either::Right((_sleep_fut, _rx_res)) => {
                    tracing::debug!("Timeout is closed without notifying");
                    return;
                }
            };
        }
    }
}
