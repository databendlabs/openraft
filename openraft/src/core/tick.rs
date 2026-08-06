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
pub(crate) const TICK_JITTER_PERCENT: u32 = 20;

/// Emit RaftMsg::Tick events at regular `interval.start`.
pub(crate) struct Tick<C>
where C: RaftTypeConfig
{
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

    /// Sample the first wait from the configured interval range.
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
        let mut delay = Self::sample_first_wait(self.interval);

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

            delay = period;

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

    /// `sample_first_wait()` draws reproducible delays without rounding away precision.
    #[test]
    fn test_sample_first_wait() {
        const SEED: u64 = 7;
        const SAMPLES: usize = 8;

        let intervals = [
            Duration::from_millis(100)..Duration::from_millis(200),
            Duration::from_millis(100)..Duration::from_micros(100_001),
            Duration::from_secs(10)..Duration::from_secs(11),
        ];

        for interval in intervals {
            let sampled = run_seeded(SEED, || {
                (0..SAMPLES)
                    .map(|_| Tick::<SeededTickConfig>::sample_first_wait(interval.clone()))
                    .collect::<Vec<_>>()
            });

            let expected = run_seeded(SEED, || {
                (0..SAMPLES).map(|_| SeededRuntime::thread_rng().random_range(interval.clone())).collect::<Vec<_>>()
            });

            assert_eq!(
                expected, sampled,
                "the whole sequence must be reproducible; interval={interval:?}"
            );
            assert!(
                sampled.iter().all(|d| interval.contains(d)),
                "{sampled:?} must all fall inside {interval:?}"
            );
            assert!(
                sampled.iter().any(|d| *d != interval.start),
                "draws must not collapse to the range start: {sampled:?}"
            );
        }

        let interval = Duration::from_secs(1)..Duration::from_secs(1);
        let sampled = run_seeded(SEED, || Tick::<SeededTickConfig>::sample_first_wait(interval));
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

            TickUTConfig::timeout(Duration::from_millis(500), join_handle)
                .await
                .expect("tick loop must stop while still inside the first wait")
                .expect("tick loop must not panic");
            assert!(rx.recv().await.is_none(), "no tick should precede the first wait");
        });
    }

    #[test]
    fn test_period_after_first_wait() {
        TickUTConfig::run(async {
            let (tx, mut rx) = TickUTConfig::mpsc(1024);
            let interval = Duration::from_millis(100);
            let th = Tick::<TickUTConfig>::spawn(interval..interval * 2, tx, true);

            let first = TickUTConfig::timeout(interval * 3, recv_tick(&mut rx)).await.unwrap();
            let early_second = TickUTConfig::timeout(interval / 2, recv_tick(&mut rx)).await;
            let second = TickUTConfig::timeout(interval, recv_tick(&mut rx)).await.unwrap();

            assert_eq!(1, first);
            assert!(early_second.is_err(), "the second tick must wait for a full period");
            assert_eq!(2, second);

            th.shutdown().unwrap().await.unwrap();
            assert!(rx.recv().await.is_none(), "the channel must close after shutdown");
        });
    }
}
