//! tick emitter emits a `RaftMsg::Tick` event at a certain interval.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::future::Either;
use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::RaftTypeConfig;
use crate::core::notification::Notification;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::async_runtime::mpsc::MpscSender;
use crate::type_config::async_runtime::oneshot::OneshotSender;

/// Emit RaftMsg::Tick event at regular `interval`.
pub(crate) struct Tick<C>
where C: RaftTypeConfig
{
    interval: Duration,

    tx: MpscSenderOf<C, Notification<C>>,

    /// Emit event or not
    enabled: Arc<AtomicBool>,

    /// Whether to apply random jitter before the first tick
    jitter: bool,
}

pub(crate) struct TickHandle<C>
where C: RaftTypeConfig
{
    enabled: Arc<AtomicBool>,
    shutdown: Mutex<Option<OneshotSenderOf<C, ()>>>,
    join_handle: Mutex<Option<JoinHandleOf<C, ()>>>,
}

impl<C> Drop for TickHandle<C>
where C: RaftTypeConfig
{
    /// Signal the tick loop to stop, without waiting for it to stop.
    fn drop(&mut self) {
        if self.shutdown.lock().unwrap().is_none() {
            return;
        }
        let _ = self.shutdown();
    }
}

impl<C> Tick<C>
where C: RaftTypeConfig
{
    pub(crate) fn spawn(
        interval: Duration,
        tx: MpscSenderOf<C, Notification<C>>,
        enabled: bool,
        jitter: bool,
    ) -> TickHandle<C> {
        let enabled = Arc::new(AtomicBool::from(enabled));
        let this = Self {
            interval,
            enabled: enabled.clone(),
            tx,
            jitter,
        };

        let (shutdown, shutdown_rx) = C::oneshot();

        let shutdown = Mutex::new(Some(shutdown));

        let join_handle = C::spawn(this.tick_loop(shutdown_rx).instrument(tracing::span!(
            parent: &Span::current(),
            Level::DEBUG,
            "tick"
        )));

        TickHandle {
            enabled,
            shutdown,
            join_handle: Mutex::new(Some(join_handle)),
        }
    }

    pub(crate) async fn tick_loop(self, cancel_rx: OneshotReceiverOf<C, ()>) {
        if self.jitter {
            use rand::RngExt;

            use crate::async_runtime::AsyncRuntime;

            let interval_ms = self.interval.as_millis() as u64;
            if interval_ms > 0 {
                let jitter_ms = C::AsyncRuntime::thread_rng().random_range(0..interval_ms);
                C::sleep(Duration::from_millis(jitter_ms)).await;
            }
        }

        let mut i = 0;

        let mut cancel = std::pin::pin!(cancel_rx);

        loop {
            let at = C::now() + self.interval;
            let sleep_fut = std::pin::pin!(C::sleep_until(at));
            let cancel_fut = cancel.as_mut();

            match futures_util::future::select(cancel_fut, sleep_fut).await {
                Either::Left((_canceled, _)) => {
                    tracing::info!("TickLoop received cancel signal, quit");
                    return;
                }
                Either::Right((_, _)) => {
                    // sleep done
                }
            }

            if !self.enabled.load(Ordering::Relaxed) {
                continue;
            }

            i += 1;

            let send_res = self.tx.send(Notification::Tick { i }).await;
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

    /// Signal the tick loop to stop. And return a JoinHandle to wait for the loop to stop.
    ///
    /// If it is called twice, the second call will return None.
    pub(crate) fn shutdown(&self) -> Option<JoinHandleOf<C, ()>> {
        {
            let shutdown = {
                let mut x = self.shutdown.lock().unwrap();
                x.take()
            };

            if let Some(shutdown) = shutdown {
                let send_res = shutdown.send(());
                tracing::info!("Timer shutdown signal sent: {send_res:?}");
            } else {
                tracing::warn!("Double call to Raft::shutdown()");
            }
        }

        {
            let mut x = self.join_handle.lock().unwrap();
            x.take()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use openraft_rt_tokio::TokioRuntime;

    use crate::OptionalSend;
    use crate::RaftTypeConfig;
    use crate::async_runtime::MpscReceiver;
    use crate::core::Tick;
    use crate::type_config::TypeConfigExt;

    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
    #[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
    pub(crate) struct TickUTConfig {}
    impl RaftTypeConfig for TickUTConfig {
        type D = u64;
        type R = ();
        type NodeId = u64;
        type Node = ();
        type Term = u64;
        type LeaderId = crate::impls::leader_id_adv::LeaderId<u64, u64>;
        type Vote = crate::impls::Vote<Self::LeaderId>;
        type Entry =
            crate::Entry<<Self::LeaderId as crate::vote::RaftLeaderId>::Committed, Self::D, Self::NodeId, Self::Node>;
        type AsyncRuntime = TokioRuntime;
        type Responder<T>
            = crate::impls::OneshotResponder<Self, T>
        where T: OptionalSend + 'static;
        type Batch<T>
            = crate::impls::InlineBatch<T>
        where T: OptionalSend + 'static;
        type ErrorSource = anyerror::AnyError;
    }

    #[test]
    fn test_shutdown() {
        TickUTConfig::run(async {
            let (tx, mut rx) = TickUTConfig::mpsc(1024);
            let th = Tick::<TickUTConfig>::spawn(Duration::from_millis(100), tx, true, true);

            TickUTConfig::sleep(Duration::from_millis(500)).await;
            th.shutdown().unwrap().await.ok();
            TickUTConfig::sleep(Duration::from_millis(500)).await;

            let mut received = vec![];
            while let Some(x) = rx.recv().await {
                received.push(x);
            }

            assert!(
                received.len() < 10,
                "no more tick will be received after shutdown: {}",
                received.len()
            );
        });
    }

    /// Collect first-tick arrivals from `n` Tick instances into 5ms buckets
    /// and return (max_bucket_count, non_empty_bucket_count).
    async fn measure_tick_clustering(n: usize, interval: Duration, jitter: bool) -> (usize, usize) {
        use openraft_rt_tokio::TokioRuntime as RT;

        use crate::async_runtime::AsyncRuntime;

        let (tx, mut rx) = TickUTConfig::mpsc(n * 4);

        let handles: Vec<_> = (0..n).map(|_| Tick::<TickUTConfig>::spawn(interval, tx.clone(), true, jitter)).collect();
        drop(tx);

        let bucket_width = Duration::from_millis(5);
        let n_buckets = (interval.as_millis() as usize / bucket_width.as_millis() as usize) + 1;
        let mut buckets = vec![0usize; n_buckets];

        let mut total_received = 0usize;
        let start = std::time::Instant::now();

        while total_received < n {
            let sleep = std::pin::pin!(RT::sleep(bucket_width));
            let recv = std::pin::pin!(rx.recv());

            match futures_util::future::select(recv, sleep).await {
                futures_util::future::Either::Left((Some(_), _)) => {
                    let elapsed = start.elapsed();
                    let offset = elapsed.as_millis().saturating_sub(interval.as_millis()) as usize;
                    let bucket_idx = (offset / bucket_width.as_millis() as usize).min(n_buckets - 1);
                    buckets[bucket_idx] += 1;
                    total_received += 1;
                }
                futures_util::future::Either::Left((None, _)) => break,
                futures_util::future::Either::Right(_) => {
                    if start.elapsed() > interval * 3 {
                        break;
                    }
                }
            }
        }

        for h in &handles {
            h.shutdown();
        }

        let max_bucket = *buckets.iter().max().unwrap_or(&0);
        let non_empty_buckets = buckets.iter().filter(|&&b| b > 0).count();
        (max_bucket, non_empty_buckets)
    }

    /// Verify that without jitter all ticks cluster into a few buckets,
    /// and with jitter they spread across many buckets.
    #[test]
    fn test_tick_jitter_comparison() {
        TickUTConfig::run(async {
            let n = 64;
            let interval = Duration::from_millis(100);

            let (no_jitter_max, no_jitter_buckets) = measure_tick_clustering(n, interval, false).await;
            let (jitter_max, jitter_buckets) = measure_tick_clustering(n, interval, true).await;

            // Without jitter: all 64 ticks fire at the same time → land in 1-2 buckets
            assert!(
                no_jitter_buckets <= 3,
                "Without jitter ticks should cluster into very few buckets, got {}",
                no_jitter_buckets
            );
            assert!(
                no_jitter_max >= n / 2,
                "Without jitter the peak bucket should hold most ticks, got {}/{}",
                no_jitter_max,
                n
            );

            // With jitter: ticks spread across many buckets
            assert!(
                jitter_max < n / 2,
                "With jitter no single bucket should hold half the ticks, got {}/{}",
                jitter_max,
                n
            );
            assert!(
                jitter_buckets >= 4,
                "With jitter ticks should spread across many buckets, got {}",
                jitter_buckets
            );

            // The jitter run must be significantly less clustered than the no-jitter run
            assert!(
                jitter_max < no_jitter_max,
                "Jitter should reduce peak clustering: jitter_max={} should be < no_jitter_max={}",
                jitter_max,
                no_jitter_max
            );
        });
    }
}
