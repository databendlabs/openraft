//! tick emitter emits a `RaftMsg::Tick` event at a certain interval.

use std::ops::Range;
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

/// Percentage of the tick interval that the first tick may be delayed by.
///
/// Raft instances created in a tight loop start their tick loops at the same instant and keep
/// waking together for the lifetime of the cluster. Drawing one offset from this fraction of the
/// interval spreads them apart permanently. See: <https://github.com/databendlabs/openraft/issues/1959>
pub(crate) const TICK_JITTER_PERCENT: u32 = 20;

/// Emit RaftMsg::Tick event at regular `interval.start`.
pub(crate) struct Tick<C>
where C: RaftTypeConfig
{
    /// Window the first tick lands in; its floor is the period between all later ticks.
    interval: Range<Duration>,

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
    /// Emit a tick every `interval.start`, with the first tick landing somewhere in `interval`.
    ///
    /// The offset drawn for that first tick persists as this instance's phase; every later tick
    /// keeps the `interval.start` period. Pass a degenerate range to start all instances in phase.
    pub(crate) fn spawn(
        interval: Range<Duration>,
        tx: MpscSenderOf<C, Notification<C>>,
        enabled: bool,
    ) -> TickHandle<C> {
        let enabled = Arc::new(AtomicBool::from(enabled));
        let this = Self {
            interval,
            enabled: enabled.clone(),
            tx,
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

    /// Sample when the first tick lands, uniformly over the window `interval`.
    ///
    /// A degenerate window yields its floor, starting every instance in phase. The sampler itself
    /// rejects an empty range, so that case is handled here.
    fn sample_first_wait(interval: Range<Duration>) -> Duration {
        if interval.is_empty() {
            return interval.start;
        }
        AsyncRuntimeOf::<C>::thread_rng().random_range(interval)
    }

    pub(crate) async fn tick_loop(self, cancel_rx: OneshotReceiverOf<C, ()>) {
        let mut i = 0;

        let mut cancel = std::pin::pin!(cancel_rx);

        let period = self.interval.start;

        // The first deadline carries this instance's phase. It is awaited through the same
        // cancellation select as every later one, so shutdown is not delayed by it.
        let mut at = C::now() + Self::sample_first_wait(self.interval);

        loop {
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

    /// `sample_first_wait()` draws one delay per call from the window it is given, reproducibly for
    /// a given seed and without rounding the window away.
    #[test]
    fn test_sample_first_wait() {
        const SEED: u64 = 7;
        const SAMPLES: usize = 8;

        let windows = [
            Duration::from_millis(100)..Duration::from_millis(200),
            // Narrower than a millisecond: draws here are tens of nanoseconds apart.
            Duration::from_millis(100)..Duration::from_micros(100_001),
            Duration::from_secs(10)..Duration::from_secs(11),
        ];

        for window in windows {
            let sampled = run_seeded(SEED, || {
                (0..SAMPLES).map(|_| Tick::<SeededTickConfig>::sample_first_wait(window.clone())).collect::<Vec<_>>()
            });

            // One `thread_rng()` draw uniformly over the window per call, no rounding step.
            let expected = run_seeded(SEED, || {
                (0..SAMPLES).map(|_| SeededRuntime::thread_rng().random_range(window.clone())).collect::<Vec<_>>()
            });

            assert_eq!(
                expected, sampled,
                "the whole sequence must be reproducible; window={window:?}"
            );
            assert!(
                sampled.iter().all(|d| window.contains(d)),
                "{sampled:?} must all fall inside {window:?}"
            );
            assert!(
                sampled.iter().any(|d| *d != window.start),
                "draws must spread across {window:?} instead of collapsing to its floor: {sampled:?}"
            );
        }

        // A degenerate window yields its floor. It must not reach the RNG, which panics on one.
        let degenerate = Duration::from_secs(1)..Duration::from_secs(1);
        let sampled = run_seeded(SEED, || Tick::<SeededTickConfig>::sample_first_wait(degenerate));
        assert_eq!(Duration::from_secs(1), sampled);

        // An inverted window is empty too, and must not underflow into a huge delay.
        let inverted = Duration::from_secs(1)..Duration::from_millis(500);
        let sampled = run_seeded(SEED, || Tick::<SeededTickConfig>::sample_first_wait(inverted));
        assert_eq!(Duration::from_secs(1), sampled);
    }

    /// Receive the next notification, asserting it is a tick, and return its number.
    async fn recv_tick(rx: &mut MpscReceiverOf<TickUTConfig, Notification<TickUTConfig>>) -> u64 {
        match rx.recv().await {
            Some(Notification::Tick { i }) => i,
            Some(other) => unreachable!("expect a Tick notification, got: {other}"),
            None => unreachable!("the tick channel closed before a tick arrived"),
        }
    }

    /// Cancelling during the first wait stops the loop at once and emits nothing; shutdown must
    /// not have to wait that first delay out.
    #[test]
    fn test_shutdown_during_first_wait() {
        TickUTConfig::run(async {
            let (tx, mut rx) = TickUTConfig::mpsc(1024);

            // A first wait far longer than this test's own timeout: the loop can only finish by
            // observing the shutdown signal.
            let th = Tick::<TickUTConfig>::spawn(Duration::from_secs(10)..Duration::from_secs(11), tx, true);

            TickUTConfig::sleep(Duration::from_millis(50)).await;
            let join_handle = th.shutdown().unwrap();

            let joined = TickUTConfig::timeout(Duration::from_millis(500), join_handle).await;
            assert!(
                joined.is_ok(),
                "tick loop must stop while still inside the first wait"
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
            let interval = Duration::from_millis(100);
            let th = Tick::<TickUTConfig>::spawn(interval..interval * 2, tx, true);

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
