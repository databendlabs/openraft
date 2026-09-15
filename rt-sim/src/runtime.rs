//! [`SimRuntime`]: the [`AsyncRuntime`] implementation.

use std::fmt;
use std::future::Future;
use std::io;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use openraft_rt::AsyncRuntime;
use openraft_rt::OptionalSend;
use rand::SeedableRng;
use rand::rngs::SmallRng;

use crate::Elapsed;
use crate::SimInstant;
use crate::SimJoinError;
use crate::SimJoinHandle;
use crate::SimMpsc;
use crate::SimMutex;
use crate::SimOneshot;
use crate::SimSleep;
use crate::SimTimeout;
use crate::SimWatch;
use crate::executor;
use crate::executor::Shared;
use crate::join::JoinSlot;
use crate::join::Task;

/// When set, every runtime appends its trace to this file when it is dropped.
pub const TRACE_ENV: &str = "OPENRAFT_RT_SIM_TRACE";

/// The seed [`AsyncRuntime::new`] uses, as a decimal `u64`. Defaults to 0.
pub const SEED_ENV: &str = "OPENRAFT_RT_SIM_SEED";

/// A deterministic, single-threaded [`AsyncRuntime`] on a virtual clock.
///
/// - Tasks are polled FIFO, in the order they became runnable, on the thread that calls
///   [`block_on`](AsyncRuntime::block_on).
/// - When nothing is runnable, virtual time jumps to the earliest pending timer. Timers sharing a
///   deadline fire in registration order.
/// - [`thread_rng`](AsyncRuntime::thread_rng) draws from a stream derived from the seed.
/// - If the main future is pending, nothing is runnable and no timer is pending, `block_on` panics.
///
/// Spawned tasks that outlive a `block_on` call stay in the runtime and continue in the next one.
pub struct SimRuntime {
    shared: Arc<Shared>,
}

impl SimRuntime {
    /// A runtime whose `thread_rng()` streams derive from `seed`.
    pub fn with_seed(seed: u64) -> Self {
        let shared = Shared::new(seed);
        shared.set_record(std::env::var_os(TRACE_ENV).is_some());
        SimRuntime { shared }
    }

    /// Starts or stops recording the trace that [`take_trace`](Self::take_trace) returns.
    /// Recording is on when [`TRACE_ENV`] is set, and off otherwise: it formats an event for every
    /// poll, which long integration tests notice.
    pub fn record_trace(&mut self, record: bool) {
        self.shared.set_record(record);
    }

    /// Changes the seed for later `thread_rng()` calls.
    pub fn set_seed(&mut self, seed: u64) {
        self.shared.set_seed(seed);
    }

    /// Returns the events recorded so far and clears them: spawns, polls, wakes, timer
    /// registrations, cancellations and fires, clock advances and RNG draws.
    pub fn take_trace(&self) -> Vec<String> {
        self.shared.take_trace()
    }
}

impl fmt::Debug for SimRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimRuntime").finish_non_exhaustive()
    }
}

impl Drop for SimRuntime {
    fn drop(&mut self) {
        self.shared.drop_tasks();

        let Some(path) = std::env::var_os(TRACE_ENV) else {
            return;
        };
        let trace = self.shared.take_trace();
        let written = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut file| {
            writeln!(file, "== runtime")?;
            for event in &trace {
                writeln!(file, "{event}")?;
            }
            Ok(())
        });
        if let Err(err) = written {
            eprintln!("rt-sim: cannot write trace: {err}");
        }
    }
}

impl AsyncRuntime for SimRuntime {
    type JoinError = SimJoinError;
    type JoinHandle<T: OptionalSend + 'static> = SimJoinHandle<T>;
    type Sleep = SimSleep;
    type Instant = SimInstant;
    type TimeoutError = Elapsed;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = SimTimeout<T>;
    type ThreadLocalRng = SmallRng;

    #[track_caller]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        let shared = executor::current();
        let slot = JoinSlot::new();
        shared.spawn(Box::pin(Task::new(future, slot.clone())));
        SimJoinHandle::new(slot)
    }

    #[track_caller]
    fn sleep(duration: Duration) -> Self::Sleep {
        Self::sleep_until(<SimInstant as openraft_rt::Instant>::now() + duration)
    }

    fn sleep_until(deadline: Self::Instant) -> Self::Sleep {
        SimSleep::until(deadline.nanos())
    }

    #[track_caller]
    fn timeout<R, F>(duration: Duration, future: F) -> Self::Timeout<R, F>
    where F: Future<Output = R> + OptionalSend {
        SimTimeout::new(future, Self::sleep(duration))
    }

    fn timeout_at<R, F>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F>
    where F: Future<Output = R> + OptionalSend {
        SimTimeout::new(future, Self::sleep_until(deadline))
    }

    fn is_panic(join_error: &Self::JoinError) -> bool {
        matches!(join_error, SimJoinError::Panic(_))
    }

    #[track_caller]
    fn thread_rng() -> Self::ThreadLocalRng {
        let seed = executor::current().lock().next_rng_seed();
        SmallRng::seed_from_u64(seed)
    }

    type Mpsc = SimMpsc;
    type Watch = SimWatch;
    type Oneshot = SimOneshot;
    type Mutex<T: OptionalSend + 'static> = SimMutex<T>;

    /// `threads` is ignored: the runtime always runs on the calling thread. The seed comes from
    /// [`SEED_ENV`] if it is set.
    fn new(_threads: usize) -> Self {
        let seed = match std::env::var(SEED_ENV) {
            Ok(value) => value.parse().unwrap_or_else(|_| panic!("rt-sim: {SEED_ENV}={value:?} is not a u64")),
            Err(_) => 0,
        };
        Self::with_seed(seed)
    }

    fn block_on<F, T>(&mut self, future: F) -> T
    where
        F: Future<Output = T>,
        T: OptionalSend,
    {
        executor::block_on(&self.shared, future)
    }

    /// Runs `f` inline, before returning. A thread would make its timing non-deterministic.
    fn spawn_blocking<F, T>(f: F) -> impl Future<Output = Result<T, io::Error>> + Send
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        std::future::ready(Ok(f()))
    }
}
