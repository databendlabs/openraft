//! tick emitter emits a `RaftMsg::Tick` event at a certain interval.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures::future::Either;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::core::notify::Notify;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::JoinHandleOf;
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
    cancel: Mutex<Option<oneshot::Sender<()>>>,
    #[allow(dead_code)]
    join_handle: JoinHandleOf<C, ()>,
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

        let (cancel, cancel_rx) = oneshot::channel();

        let join_handle = AsyncRuntimeOf::<C>::spawn(this.tick_loop(cancel_rx).instrument(tracing::span!(
            parent: &Span::current(),
            Level::DEBUG,
            "tick"
        )));
        TickHandle {
            enabled,
            cancel: Mutex::new(Some(cancel)),
            join_handle,
        }
    }

    pub(crate) async fn tick_loop(self, mut cancel_rx: oneshot::Receiver<()>) {
        let mut i = 0;

        let mut cancel = std::pin::pin!(cancel_rx);

        loop {
            i += 1;

            let at = <C::AsyncRuntime as AsyncRuntime>::Instant::now() + self.interval;
            let mut sleep_fut = AsyncRuntimeOf::<C>::sleep_until(at);
            let sleep_fut = std::pin::pin!(sleep_fut);
            let cancel_fut = cancel.as_mut();

            match futures::future::select(cancel_fut, sleep_fut).await {
                Either::Left((_canceled, _)) => {
                    tracing::info!("TickLoop received cancel signal, quit");
                    return;
                }
                Either::Right((_, _)) => {
                    // sleep done
                }
            }

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
        let got = {
            let mut x = self.cancel.lock().unwrap();
            x.take()
        };

        if let Some(cancel) = got {
            let send_res = cancel.send(());
            tracing::info!("Timer cancel signal is sent, result is ok: {}", send_res.is_ok());
        } else {
            tracing::info!("Timer cancel signal is already sent");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use tokio::time::Duration;

    use crate::core::Tick;
    use crate::type_config::alias::AsyncRuntimeOf;
    use crate::AsyncRuntime;
    use crate::RaftTypeConfig;
    use crate::TokioRuntime;

    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
    #[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
    pub(crate) struct TickUTConfig {}
    impl RaftTypeConfig for TickUTConfig {
        type D = ();
        type R = ();
        type NodeId = u64;
        type Node = ();
        type Entry = crate::Entry<TickUTConfig>;
        type SnapshotData = Cursor<Vec<u8>>;
        type AsyncRuntime = TokioRuntime;
    }

    // AsyncRuntime::spawn is `spawn_local` with singlethreaded enabled.
    // It will result in a panic:
    // `spawn_local` called from outside of a `task::LocalSet`.
    #[cfg(not(feature = "singlethreaded"))]
    #[tokio::test]
    async fn test_shutdown() -> anyhow::Result<()> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let th = Tick::<TickUTConfig>::spawn(Duration::from_millis(100), tx, true);

        AsyncRuntimeOf::<TickUTConfig>::sleep(Duration::from_millis(500)).await;
        th.shutdown().await;
        AsyncRuntimeOf::<TickUTConfig>::sleep(Duration::from_millis(500)).await;

        let mut received = vec![];
        while let Some(x) = rx.recv().await {
            received.push(x);
        }

        assert!(
            received.len() < 10,
            "no more tick will be received after shutdown: {}",
            received.len()
        );

        Ok(())
    }
}
