//! tick emitter emits a `RaftMsg::Tick` event at a certain interval.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::sleep_until;
use tokio::time::Instant;
use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::raft::RaftMsg;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;

/// Emit RaftMsg::Tick event at regular `interval`.
pub(crate) struct Tick<C, N, S>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    S: RaftStorage<C>,
{
    interval: Duration,

    tx: mpsc::UnboundedSender<RaftMsg<C, N, S>>,

    /// Emit event or not
    enabled: Arc<AtomicBool>,
}

pub(crate) struct TickHandle {
    enabled: Arc<AtomicBool>,
    join_handle: JoinHandle<()>,
}

impl<C, N, S> Tick<C, N, S>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    S: RaftStorage<C>,
{
    pub(crate) fn spawn(interval: Duration, tx: mpsc::UnboundedSender<RaftMsg<C, N, S>>, enabled: bool) -> TickHandle {
        let enabled = Arc::new(AtomicBool::from(enabled));
        let this = Self {
            interval,
            enabled: enabled.clone(),
            tx,
        };
        let join_handle =
            tokio::spawn(this.tick_loop().instrument(tracing::span!(parent: &Span::current(), Level::DEBUG, "tick")));
        TickHandle { enabled, join_handle }
    }

    pub(crate) async fn tick_loop(self) {
        let mut i = 0;
        loop {
            i += 1;

            let at = Instant::now() + self.interval;
            sleep_until(at).await;

            if !self.enabled.load(Ordering::Relaxed) {
                i -= 1;
                continue;
            }

            let send_res = self.tx.send(RaftMsg::Tick { i });
            if let Err(e) = send_res {
                tracing::info!("Tick fails to send, receiving end quit: {e}");
            } else {
                tracing::debug!("Tick sent: {}", i)
            }
        }
    }
}

impl TickHandle {
    pub(crate) fn enable(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub(crate) async fn shutdown(&self) {
        self.join_handle.abort();
    }
}
