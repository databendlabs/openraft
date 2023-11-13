//! tick emitter emits a `RaftMsg::Tick` event at a certain interval.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::core::notify::Notify;
use crate::AsyncRuntime;
use crate::Instant;
use crate::RaftTypeConfig;

/// Emit RaftMsg::Tick event at regular `interval`.
pub(crate) struct Tick<C>
where C: RaftTypeConfig
{
    interval: Duration,

    tx: mpsc::UnboundedSender<Notify<C>>,

    /// Emit event or not
    enabled: Arc<AtomicBool>,
}

pub(crate) struct TickHandle<C>
where C: RaftTypeConfig
{
    enabled: Arc<AtomicBool>,
    join_handle: <C::AsyncRuntime as AsyncRuntime>::JoinHandle<()>,
}

impl<C> Tick<C>
where C: RaftTypeConfig
{
    pub(crate) fn spawn(interval: Duration, tx: mpsc::UnboundedSender<Notify<C>>, enabled: bool) -> TickHandle<C> {
        let enabled = Arc::new(AtomicBool::from(enabled));
        let this = Self {
            interval,
            enabled: enabled.clone(),
            tx,
        };
        let join_handle = C::AsyncRuntime::spawn(this.tick_loop().instrument(tracing::span!(
            parent: &Span::current(),
            Level::DEBUG,
            "tick"
        )));
        TickHandle { enabled, join_handle }
    }

    pub(crate) async fn tick_loop(self) {
        let mut i = 0;
        loop {
            i += 1;

            let at = <C::AsyncRuntime as AsyncRuntime>::Instant::now() + self.interval;
            C::AsyncRuntime::sleep_until(at).await;

            if !self.enabled.load(Ordering::Relaxed) {
                i -= 1;
                continue;
            }

            let send_res = self.tx.send(Notify::Tick { i });
            if let Err(_e) = send_res {
                tracing::info!("Stopping tick_loop(), main loop terminated");
                break;
            } else {
                tracing::debug!("Tick sent: {}", i)
            }
        }
    }
}

impl<C> TickHandle<C>
where C: RaftTypeConfig
{
    pub(crate) fn enable(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub(crate) async fn shutdown(&self) {
        C::AsyncRuntime::abort(&self.join_handle);
    }
}
