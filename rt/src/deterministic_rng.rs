//! Deterministic RNG wrapper for async runtimes.
//!
//! [`DeterministicRng<RT>`] wraps any [`AsyncRuntime`] and replaces its RNG with a
//! deterministic, seed-based [`SmallRng`]. Each runtime instance maintains its own
//! seed counter via task-local storage, ensuring independent deterministic sequences
//! even when multiple runtimes coexist.
//!
//! ## Seed derivation
//!
//! Spawning forms a tree. To avoid seed collisions between parent and child
//! tasks, two different mixing functions are used:
//!
//! - **spawn**: `child.seed = f(parent.seed)`, `parent.seed = g(parent.seed)`
//! - **thread_rng**: uses `parent.seed` directly, `parent.seed = g(parent.seed)`
//!
//! `f` and `g` are both SplitMix64 bijective mixers with different additive
//! constants, so the child's seed tree diverges from the parent's at every fork.

use std::cell::Cell;
use std::fmt::Debug;
use std::future::Future;
use std::io;
use std::time::Duration;

use rand::SeedableRng;
use rand::rngs::SmallRng;

use crate::AsyncRuntime;
use crate::OptionalSend;

/// Golden ratio fractional bits — used by `derive_spawn_seed`.
const GOLDEN_RATIO: u64 = 0x9e3779b97f4a7c15;

/// sqrt(2) fractional bits — used by `advance_seed`.
const SQRT2: u64 = 0x6a09e667f3bcc908;

task_local! {
    static DETSIM_SEED: Cell<u64>;
}

/// SplitMix64 bijective mixer.
///
/// A composition of add, xor-right-shift, and multiply — each a bijection
/// on u64 — producing excellent avalanche (each input bit affects ~half
/// the output bits). The `gamma` constant determines which Weyl sequence
/// the mixer draws from; different gammas yield independent sequences.
fn splitmix64(seed: u64, gamma: u64) -> u64 {
    let z = seed.wrapping_add(gamma);
    let z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/// Derive the child task's seed from the parent's current seed.
fn derive_spawn_seed(seed: u64) -> u64 {
    splitmix64(seed, GOLDEN_RATIO)
}

/// Advance the parent's seed after a spawn or `thread_rng` call.
fn advance_seed(seed: u64) -> u64 {
    splitmix64(seed, SQRT2)
}

/// Read the current task-local seed, advance it, and return the old value.
fn take_and_advance_seed() -> u64 {
    DETSIM_SEED.with(|c| {
        let seed = c.get();
        c.set(advance_seed(seed));
        seed
    })
}

/// A deterministic RNG wrapper around any [`AsyncRuntime`].
///
/// Replaces the inner runtime's thread-local RNG with a deterministic
/// [`SmallRng`]. Each instance has its own seed counter, stored in
/// task-local storage via `block_on`, so multiple instances produce
/// independent deterministic sequences.
pub struct DeterministicRng<RT>
where RT: AsyncRuntime
{
    inner: RT,
    seed: u64,
}

impl<RT> DeterministicRng<RT>
where RT: AsyncRuntime
{
    /// Set the seed for this runtime instance.
    ///
    /// Call this before `block_on` to ensure deterministic behavior.
    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
    }
}

impl<RT> Debug for DeterministicRng<RT>
where RT: AsyncRuntime
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeterministicRng").field("inner", &self.inner).finish()
    }
}

impl<RT> AsyncRuntime for DeterministicRng<RT>
where RT: AsyncRuntime
{
    type JoinError = RT::JoinError;
    type JoinHandle<T: OptionalSend + 'static> = RT::JoinHandle<T>;
    type Sleep = RT::Sleep;
    type Instant = RT::Instant;
    type TimeoutError = RT::TimeoutError;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = RT::Timeout<R, T>;
    type ThreadLocalRng = SmallRng;

    #[inline]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        let child_seed = derive_spawn_seed(take_and_advance_seed());
        RT::spawn(DETSIM_SEED.scope(Cell::new(child_seed), future))
    }

    #[inline]
    fn sleep(duration: Duration) -> Self::Sleep {
        RT::sleep(duration)
    }

    #[inline]
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep {
        RT::sleep_until(deadline)
    }

    #[inline]
    fn timeout<R, F>(duration: Duration, future: F) -> Self::Timeout<R, F>
    where F: Future<Output = R> + OptionalSend {
        RT::timeout(duration, future)
    }

    #[inline]
    fn timeout_at<R, F>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F>
    where F: Future<Output = R> + OptionalSend {
        RT::timeout_at(deadline, future)
    }

    #[inline]
    fn is_panic(join_error: &Self::JoinError) -> bool {
        RT::is_panic(join_error)
    }

    #[inline]
    fn thread_rng() -> Self::ThreadLocalRng {
        SmallRng::seed_from_u64(take_and_advance_seed())
    }

    type Mpsc = RT::Mpsc;
    type Watch = RT::Watch;
    type Oneshot = RT::Oneshot;
    type Mutex<T: OptionalSend + 'static> = RT::Mutex<T>;

    fn new(threads: usize) -> Self {
        DeterministicRng {
            inner: RT::new(threads),
            seed: 0,
        }
    }

    fn block_on<F, T>(&mut self, future: F) -> T
    where
        F: Future<Output = T>,
        T: OptionalSend,
    {
        DETSIM_SEED.sync_scope(Cell::new(self.seed), || {
            let result = self.inner.block_on(future);
            self.seed = DETSIM_SEED.with(|c| c.get());
            result
        })
    }

    fn spawn_blocking<F, T>(f: F) -> impl Future<Output = Result<T, io::Error>> + Send
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        RT::spawn_blocking(f)
    }
}
