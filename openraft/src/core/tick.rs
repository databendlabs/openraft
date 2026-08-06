//! tick emitter emits a `RaftMsg::Tick` event at a certain interval.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::future::Either;
use rand::RngExt;
use tracing::Instrument;
use tracing::Level;
use tracing::Span;

use crate::AsyncRuntime;
use crate::RaftTypeConfig;
use crate::core::notification::Notification;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::AsyncRuntimeOf;
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
    pub(crate) fn spawn(interval: Duration, tx: MpscSenderOf<C, Notification<C>>, enabled: bool) -> TickHandle<C> {
        let enabled = Arc::new(AtomicBool::from(enabled));
        let this = Self {
            interval,
            enabled: enabled.clone(),
            tx,
        };

        let (shutdown, shutdown_rx) = C::oneshot();

        let shutdown = Mutex::new(Some(shutdown));

        let initial_jitter = Self::sample_jitter(interval);

        let join_handle = C::spawn(this.tick_loop(initial_jitter, shutdown_rx).instrument(tracing::span!(
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

    /// Sample a one-time phase offset for the first tick, uniformly from `[0, interval)`.
    ///
    /// Raft instances created in a tight loop start their tick loops at the same instant, land in
    /// the same timer slot, and keep waking together for the lifetime of the cluster. Delaying the
    /// first tick by a random fraction of one interval spreads the instances across the interval,
    /// and because only the first wait is extended the offset persists as the instance's phase.
    ///
    /// See: <https://github.com/databendlabs/openraft/issues/1959>
    fn sample_jitter(interval: Duration) -> Duration {
        if interval.is_zero() {
            return Duration::ZERO;
        }
        AsyncRuntimeOf::<C>::thread_rng().random_range(Duration::ZERO..interval)
    }

    pub(crate) async fn tick_loop(self, initial_jitter: Duration, cancel_rx: OneshotReceiverOf<C, ()>) {
        let mut i = 0;

        let mut cancel = std::pin::pin!(cancel_rx);

        // Only the first wait carries the phase offset, and it goes through the same cancellation
        // select as every later wait, so shutdown is not delayed by the offset.
        let mut delay = self.interval.saturating_add(initial_jitter);

        loop {
            let at = C::now() + delay;
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

            delay = self.interval;

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
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use openraft_rt::deterministic_rng::DeterministicRng;
    use openraft_rt_tokio::TokioRuntime;
    use rand::RngExt;

    use crate::AsyncRuntime;
    use crate::OptionalSend;
    use crate::RaftTypeConfig;
    use crate::async_runtime::MpscReceiver;
    use crate::core::Tick;
    use crate::core::notification::Notification;
    use crate::type_config::TypeConfigExt;
    use crate::type_config::alias::MpscReceiverOf;
    use crate::type_config::async_runtime::oneshot::OneshotSender;

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

    /// A runtime whose `thread_rng()` is seeded, so sampled jitter is reproducible.
    type SeededRuntime = DeterministicRng<TokioRuntime>;

    crate::declare_raft_types!(
        SeededTickConfig:
            D = u64,
            R = (),
            Node = (),
            AsyncRuntime = SeededRuntime,
    );

    /// Run `test` with `seed` installed as the deterministic RNG seed.
    fn run_seeded<F, T>(seed: u64, test: F) -> T
    where
        F: FnOnce() -> T,
        T: Send,
    {
        let mut runtime = TokioRuntime::new(1);
        runtime.block_on(SeededRuntime::scope(seed, async move { test() }))
    }

    /// `sample_jitter()` draws one offset per call from `[0, interval)`, reproducibly for a given
    /// seed and without collapsing a sub-millisecond interval to zero.
    #[test]
    fn test_sample_jitter() {
        const SEED: u64 = 7;
        const SAMPLES: usize = 8;

        for interval in [Duration::from_millis(100), Duration::from_micros(1)] {
            let sampled = run_seeded(SEED, || {
                (0..SAMPLES).map(|_| Tick::<SeededTickConfig>::sample_jitter(interval)).collect::<Vec<_>>()
            });

            // One `thread_rng()` draw uniformly over `[0, interval)` per call, no rounding step.
            let expected = run_seeded(SEED, || {
                (0..SAMPLES)
                    .map(|_| SeededRuntime::thread_rng().random_range(Duration::ZERO..interval))
                    .collect::<Vec<_>>()
            });

            assert_eq!(
                expected, sampled,
                "the whole sequence must be reproducible; interval={interval:?}"
            );
            assert!(
                sampled.iter().all(|d| *d < interval),
                "{sampled:?} must all be below {interval:?}"
            );
            assert!(
                sampled.iter().any(|d| !d.is_zero()),
                "a sub-millisecond interval must not collapse every offset to zero: {sampled:?}"
            );
        }

        // A zero interval leaves no room to offset. It must not reach the RNG, which panics on an
        // empty range.
        let sampled = run_seeded(SEED, || Tick::<SeededTickConfig>::sample_jitter(Duration::ZERO));
        assert_eq!(Duration::ZERO, sampled);
    }

    /// Receive the next notification, asserting it is a tick, and return its number.
    async fn recv_tick(rx: &mut MpscReceiverOf<TickUTConfig, Notification<TickUTConfig>>) -> u64 {
        match rx.recv().await {
            Some(Notification::Tick { i }) => i,
            Some(other) => unreachable!("expect a Tick notification, got: {other}"),
            None => unreachable!("the tick channel closed before a tick arrived"),
        }
    }

    /// The offset extends the first wait only: later ticks keep arriving one `interval` apart, and
    /// tick numbering is unaffected.
    #[test]
    fn test_only_the_first_wait_is_extended() {
        TickUTConfig::run(async {
            let interval = Duration::from_millis(10);
            // Two orders of magnitude above `interval`, so a later wait that still carried the
            // offset would be unmistakable even on a badly stalled host.
            let initial_jitter = Duration::from_secs(1);

            let (tx, mut rx) = TickUTConfig::mpsc(1024);
            let (_cancel_tx, cancel_rx) = TickUTConfig::oneshot();

            let tick = Tick::<TickUTConfig> {
                interval,
                tx,
                enabled: Arc::new(AtomicBool::from(true)),
            };
            let _join_handle = TickUTConfig::spawn(tick.tick_loop(initial_jitter, cancel_rx));

            let started = TickUTConfig::now();
            assert_eq!(1, recv_tick(&mut rx).await);
            let first_at = TickUTConfig::now();
            assert_eq!(2, recv_tick(&mut rx).await);
            let second_at = TickUTConfig::now();

            assert!(
                first_at - started >= interval + initial_jitter,
                "the first tick waits out one interval plus the offset, waited: {:?}",
                first_at - started
            );
            assert!(
                second_at - first_at >= interval,
                "a later wait is still at least one interval, waited: {:?}",
                second_at - first_at
            );
            assert!(
                second_at - first_at < initial_jitter,
                "a later wait must not carry the offset again, waited: {:?}",
                second_at - first_at
            );
        });
    }

    /// Cancelling during the first, jitter-extended wait stops the loop at once and emits nothing;
    /// shutdown must not have to wait the offset out.
    #[test]
    fn test_shutdown_during_initial_jitter() {
        TickUTConfig::run(async {
            let (tx, mut rx) = TickUTConfig::mpsc(1024);
            let (cancel_tx, cancel_rx) = TickUTConfig::oneshot();

            let tick = Tick::<TickUTConfig> {
                interval: Duration::from_millis(100),
                tx,
                enabled: Arc::new(AtomicBool::from(true)),
            };

            // An offset far longer than this test's own timeout: the loop can only finish by
            // observing the cancel signal.
            let join_handle = TickUTConfig::spawn(tick.tick_loop(Duration::from_secs(10), cancel_rx));

            TickUTConfig::sleep(Duration::from_millis(50)).await;
            cancel_tx.send(()).unwrap();

            let joined = TickUTConfig::timeout(Duration::from_millis(500), join_handle).await;
            assert!(
                joined.is_ok(),
                "tick loop must stop while still inside the initial jittered wait"
            );
            assert!(joined.ok().unwrap().is_ok(), "tick loop must not panic");

            let mut received = vec![];
            while let Some(x) = rx.recv().await {
                received.push(x);
            }

            assert!(
                received.is_empty(),
                "no tick before the first wait elapses, got {}",
                received.len()
            );
        });
    }

    #[test]
    fn test_shutdown() {
        TickUTConfig::run(async {
            let (tx, mut rx) = TickUTConfig::mpsc(1024);
            let th = Tick::<TickUTConfig>::spawn(Duration::from_millis(100), tx, true);

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
}
