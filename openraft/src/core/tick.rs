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

/// Emit RaftMsg::Tick events at a regular `period` after a randomized first wait.
pub(crate) struct Tick<C>
where C: RaftTypeConfig
{
    period: Duration,
    first_wait: Duration,

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
    pub(crate) fn spawn(period: Duration, tx: MpscSenderOf<C, Notification<C>>, enabled: bool) -> TickHandle<C> {
        let enabled = Arc::new(AtomicBool::from(enabled));
        let this = Self {
            period,
            first_wait: Self::sample_first_wait(period),
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

    /// Add a random phase offset from `[0, period)` to the first wait.
    fn sample_first_wait(period: Duration) -> Duration {
        if period.is_zero() {
            return period;
        }
        AsyncRuntimeOf::<C>::thread_rng().random_range(period..period * 2)
    }

    pub(crate) async fn tick_loop(self, cancel_rx: OneshotReceiverOf<C, ()>) {
        let mut i = 0;

        let mut cancel = std::pin::pin!(cancel_rx);

        let first_wait_at = C::now() + self.first_wait;
        let mut at = first_wait_at;
        // Microsecond resolution is far finer than any tick period, and the floor of one keeps
        // the modulo below defined for a zero period, which stays a busy loop as it was before.
        let step_us = self.period.as_micros().max(1);

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

            // Round the time elapsed since the origin up to the next whole multiple of the
            // period, so every deadline is `first_wait_at + k * step_us`. Deriving it from the fixed
            // origin rather than from this wake keeps wake latency out of the phase: re-anchoring
            // would stretch the effective period and let the sampled offset wander until
            // instances fell into a shared timer slot, where they would stay. Whole periods
            // missed during a stall are stepped over by the same arithmetic, so a stall cannot
            // shift the phase either, nor emit one catch-up tick per period it covered.
            let mut offset_us = (C::now() - first_wait_at).as_micros();
            offset_us -= offset_us % step_us;
            offset_us += step_us;
            at = first_wait_at + Duration::from_micros(offset_us as u64);

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
    use std::future::Future;
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
        type Payload = crate::EntryPayload<Self::D, Self::NodeId, Self::Node>;
        type Entry = crate::Entry<<Self::LeaderId as crate::vote::RaftLeaderId>::Committed, Self::Payload>;
        type AsyncRuntime = TokioRuntime;
        type Responder<T>
            = crate::impls::OneshotResponder<Self, T>
        where T: OptionalSend + 'static;
        type Batch<T>
            = crate::impls::InlineBatch<T>
        where T: OptionalSend + 'static;
        type ErrorSource = anyerror::AnyError;
    }

    /// A runtime whose `thread_rng()` is seeded, so sampled first waits are reproducible.
    type SeededRuntime = DeterministicRng<TokioRuntime>;

    crate::declare_raft_types!(
        SeededTickConfig:
            D = u64,
            R = (),
            Node = (),
            AsyncRuntime = SeededRuntime,
    );

    /// Run `future` with `seed` installed as the deterministic RNG seed.
    fn run_seeded<F, T>(seed: u64, future: F) -> T
    where
        F: Future<Output = T>,
        T: Send,
    {
        let mut runtime = TokioRuntime::new(1);
        runtime.block_on(SeededRuntime::scope(seed, future))
    }

    #[test]
    fn test_sample_first_wait_preserves_duration_precision() {
        const SEED: u64 = 7;
        const SAMPLES: usize = 8;

        let periods = [
            Duration::from_millis(100),
            Duration::from_micros(100),
            Duration::from_secs(10),
        ];

        for period in periods {
            let sampled = run_seeded(SEED, async {
                (0..SAMPLES).map(|_| Tick::<SeededTickConfig>::sample_first_wait(period)).collect::<Vec<_>>()
            });

            let expected = run_seeded(SEED, async {
                (0..SAMPLES)
                    .map(|_| SeededRuntime::thread_rng().random_range(period..period * 2))
                    .collect::<Vec<_>>()
            });

            assert_eq!(
                expected, sampled,
                "the whole sequence must be reproducible; period={period:?}"
            );
            assert!(
                sampled.iter().all(|d| (period..period * 2).contains(d)),
                "{sampled:?} must all fall inside the first-wait range for {period:?}"
            );
            assert!(
                sampled.iter().any(|d| *d != period),
                "draws must not collapse to the period: {sampled:?}"
            );
        }

        let sampled = run_seeded(SEED, async {
            Tick::<SeededTickConfig>::sample_first_wait(Duration::ZERO)
        });
        assert_eq!(Duration::ZERO, sampled);
    }

    #[test]
    fn test_sample_first_wait_is_uniformly_distributed() {
        const SEED: u64 = 11;
        const BUCKETS: usize = 10;
        const SAMPLES_PER_BUCKET: usize = 1_000;
        const MAX_DEVIATION: usize = SAMPLES_PER_BUCKET / 10;

        let period = Duration::from_millis(100);
        let sampled = run_seeded(SEED, async {
            (0..BUCKETS * SAMPLES_PER_BUCKET)
                .map(|_| Tick::<SeededTickConfig>::sample_first_wait(period))
                .collect::<Vec<_>>()
        });

        let mut counts = [0_usize; BUCKETS];
        for first_wait in sampled {
            let offset = first_wait - period;
            let bucket = (offset.as_nanos() * BUCKETS as u128 / period.as_nanos()) as usize;
            counts[bucket] += 1;
        }

        assert!(
            counts.iter().all(|count| count.abs_diff(SAMPLES_PER_BUCKET) <= MAX_DEVIATION),
            "each bucket must be within 10% of the expected count {SAMPLES_PER_BUCKET}: {counts:?}"
        );
    }

    /// Receive the next notification, asserting it is a tick, and return its number.
    async fn recv_tick<C>(rx: &mut MpscReceiverOf<C, Notification<C>>) -> u64
    where C: RaftTypeConfig {
        match rx.recv().await {
            Some(Notification::Tick { i }) => i,
            Some(other) => unreachable!("expect a Tick notification, got: {other}"),
            None => unreachable!("the tick channel closed before a tick arrived"),
        }
    }

    #[test]
    fn test_shutdown_interrupts_first_wait() {
        TickUTConfig::run(async {
            let (tx, mut rx) = TickUTConfig::mpsc(1024);

            // A first wait far longer than this test's own timeout: the loop can only finish by
            // observing the shutdown signal.
            let th = Tick::<TickUTConfig>::spawn(Duration::from_secs(10), tx, true);

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
    fn test_only_first_wait_is_randomized() {
        const SEED: u64 = 0;

        let period = Duration::from_millis(200);
        let margin = Duration::from_millis(25);
        let first_wait = run_seeded(SEED, async { Tick::<SeededTickConfig>::sample_first_wait(period) });
        assert!(
            first_wait - period > margin * 2,
            "seed must provide a measurable phase offset: {first_wait:?}"
        );

        run_seeded(SEED, async {
            let (tx, mut rx) = SeededTickConfig::mpsc(1024);
            let th = Tick::<SeededTickConfig>::spawn(period, tx, true);

            let early_first =
                SeededTickConfig::timeout(first_wait - margin, recv_tick::<SeededTickConfig>(&mut rx)).await;
            assert!(
                early_first.is_err(),
                "the first tick must include the sampled phase offset"
            );
            let first = SeededTickConfig::timeout(margin * 2, recv_tick::<SeededTickConfig>(&mut rx)).await.unwrap();

            let early_second = SeededTickConfig::timeout(period - margin, recv_tick::<SeededTickConfig>(&mut rx)).await;
            assert!(early_second.is_err(), "the second tick must wait for a full period");
            let second = SeededTickConfig::timeout(margin * 2, recv_tick::<SeededTickConfig>(&mut rx)).await.unwrap();

            assert_eq!(1, first);
            assert_eq!(2, second);

            th.shutdown().unwrap().await.unwrap();
            assert!(rx.recv().await.is_none(), "the channel must close after shutdown");
        });
    }

    /// A tick delayed inside `send()` must not push the following deadlines back.
    ///
    /// Deriving each deadline from `now()` instead would restart the period at the end of the
    /// delay, so every wake latency would accumulate into the phase and the sampled offset would
    /// wash out after a few dozen ticks.
    #[test]
    fn test_a_delayed_tick_does_not_shift_the_schedule() {
        const PERIOD: Duration = Duration::from_millis(100);
        // Outlasts the second tick, which fires before `2 * PERIOD` because `first_wait` is drawn
        // from `[PERIOD, 2 * PERIOD)`.
        const STALL: Duration = Duration::from_millis(350);

        TickUTConfig::run(async {
            // Capacity one: the first tick fills the channel, so the second blocks in `send()`
            // until this task drains it, and the tick loop is held past its own schedule.
            let (tx, mut rx) = TickUTConfig::mpsc(1);
            let th = Tick::<TickUTConfig>::spawn(PERIOD, tx, true);

            TickUTConfig::sleep(STALL).await;

            assert_eq!(1, recv_tick::<TickUTConfig>(&mut rx).await);
            assert_eq!(2, recv_tick::<TickUTConfig>(&mut rx).await);

            // The third tick kept the deadline it was given before the stall, which is at most one
            // period away; restarting the period after the stall would place it a full period out.
            let third = TickUTConfig::timeout(PERIOD * 3 / 4, recv_tick::<TickUTConfig>(&mut rx))
                .await
                .expect("the delayed send must not postpone the third tick");
            assert_eq!(3, third);

            th.shutdown().unwrap().await.unwrap();
        });
    }

    /// A zero period must keep degenerating into a busy loop rather than dividing by zero.
    ///
    /// `Config::validate()` accepts `heartbeat_interval == 0`, so the period reaches the loop.
    #[test]
    fn test_a_zero_period_keeps_ticking() {
        TickUTConfig::run(async {
            // Capacity one throttles the busy loop to this task's receive rate.
            let (tx, mut rx) = TickUTConfig::mpsc(1);
            let th = Tick::<TickUTConfig>::spawn(Duration::ZERO, tx, true);

            for expected in 1..=3 {
                let i = TickUTConfig::timeout(Duration::from_secs(1), recv_tick::<TickUTConfig>(&mut rx))
                    .await
                    .expect("a zero period must keep ticking");
                assert_eq!(expected, i);
            }

            // Dropping the receiver releases the loop if it is waiting on the full channel.
            drop(rx);
            th.shutdown().unwrap().await.unwrap();
        });
    }

    /// A stall spanning whole periods must leave the schedule on its original grid.
    ///
    /// Skipping the missed periods in whole multiples of the period preserves the sampled phase,
    /// so one stall shared by every instance cannot pull their phases together. Re-anchoring on
    /// `now()` would hand them all the same phase, and firing the missed periods instead would
    /// emit one catch-up tick each.
    #[test]
    fn test_a_stall_skips_missed_periods_and_keeps_the_phase() {
        const SEED: u64 = 0;
        const PERIOD: Duration = Duration::from_millis(100);
        // Ends half a period before the grid line at `first_wait + 5 * PERIOD`.
        const STALL: Duration = Duration::from_millis(450);

        let first_wait = run_seeded(SEED, async { Tick::<SeededTickConfig>::sample_first_wait(PERIOD) });

        run_seeded(SEED, async {
            // Capacity one: the second tick stays blocked in `send()` for the whole stall, so the
            // loop sleeps through the deadlines at `first_wait + 3 * PERIOD` and `+ 4 * PERIOD`.
            let (tx, mut rx) = SeededTickConfig::mpsc(1);
            let th = Tick::<SeededTickConfig>::spawn(PERIOD, tx, true);

            SeededTickConfig::sleep(first_wait + STALL).await;

            assert_eq!(1, recv_tick::<SeededTickConfig>(&mut rx).await);
            assert_eq!(2, recv_tick::<SeededTickConfig>(&mut rx).await);
            // The deadline the loop already held when the stall began, so it is due at once.
            assert_eq!(3, recv_tick::<SeededTickConfig>(&mut rx).await);

            // Only now does the loop see how far behind it is. The two periods it slept through
            // are skipped rather than fired, leaving the next tick half a period out.
            let early = SeededTickConfig::timeout(PERIOD / 4, recv_tick::<SeededTickConfig>(&mut rx)).await;
            assert!(early.is_err(), "the missed periods must be skipped, not emitted");

            let fourth = SeededTickConfig::timeout(PERIOD / 2, recv_tick::<SeededTickConfig>(&mut rx))
                .await
                .expect("the tick after the stall must land on the original grid");
            assert_eq!(4, fourth);

            th.shutdown().unwrap().await.unwrap();
        });
    }
}
