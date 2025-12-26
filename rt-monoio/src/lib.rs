//! This crate provides a [`MonoioRuntime`] type, which has [`AsyncRuntime`]
//! implemented so that you can use Openraft with [Monoio](monoio).
//!
//! ```ignore
//! pub struct TypeConfig {}
//!
//! impl openraft::RaftTypeConfig for TypeConfig {
//!     // Other type are omitted
//!
//!     type AsyncRuntime = openraft_rt_monoio::MonoioRuntime;
//! }
//! ```
//!
//! # NOTE
//!
//! 1. For the Openraft dependency used with this crate
//!    1. You can disable the `default` feature as you don't need the built-in Tokio runtime.
//!    2. The `single-threaded` feature needs to be enabled or this crate won't work.
//! 2. With the `single-threaded` feature enabled, the handle type [`Raft`](openraft::Raft) will be
//!    no longer [`Send`] and [`Sync`].
//! 3. Even though this crate allows you to use Monoio, it still uses some primitives from Tokio
//!    1. `Watch`: Monoio (or `local_sync`) does not have a watch channel.
//!    2. `Mutex`: Monoio does not provide a Mutex implementation.

use std::future::Future;
use std::time::Duration;

use openraft_rt::AsyncRuntime;
use openraft_rt::OptionalSend;

mod instant;
mod mpsc;
mod mutex;
mod oneshot;
mod watch;

use instant::MonoioInstant;
use mpsc::MonoioMpsc;
use mutex::TokioMutex;
use oneshot::MonoioOneshot;
use watch::TokioWatch;

/// The monoio runtime type varies by platform.
/// On Linux, FusionRuntime has two drivers (iouring + legacy fallback).
/// On other platforms, only legacy driver is available.
#[cfg(target_os = "linux")]
type InnerRuntime = monoio::FusionRuntime<
    monoio::time::TimeDriver<monoio::IoUringDriver>,
    monoio::time::TimeDriver<monoio::LegacyDriver>,
>;

#[cfg(not(target_os = "linux"))]
type InnerRuntime = monoio::FusionRuntime<monoio::time::TimeDriver<monoio::LegacyDriver>>;

/// [`AsyncRuntime`] implementation for Monoio.
pub struct MonoioRuntime {
    rt: InnerRuntime,
}

impl std::fmt::Debug for MonoioRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonoioRuntime").finish()
    }
}

impl AsyncRuntime for MonoioRuntime {
    // Joining an async task on Monoio always succeeds
    type JoinError = std::convert::Infallible;
    type JoinHandle<T: OptionalSend + 'static> = monoio::task::JoinHandle<Result<T, Self::JoinError>>;
    type Sleep = monoio::time::Sleep;
    type Instant = MonoioInstant;
    type TimeoutError = monoio::time::error::Elapsed;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = monoio::time::Timeout<T>;
    type ThreadLocalRng = rand::rngs::ThreadRng;

    #[inline]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        monoio::spawn(async move { Ok(future.await) })
    }

    #[inline]
    fn sleep(duration: Duration) -> Self::Sleep {
        monoio::time::sleep(duration)
    }

    #[inline]
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep {
        monoio::time::sleep_until(deadline.0)
    }

    #[inline]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F> {
        monoio::time::timeout(duration, future)
    }

    #[inline]
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F> {
        monoio::time::timeout_at(deadline.0, future)
    }

    #[inline]
    fn is_panic(join_error: &Self::JoinError) -> bool {
        match *join_error {}
    }

    #[inline]
    fn thread_rng() -> Self::ThreadLocalRng {
        rand::rng()
    }

    type Mpsc = MonoioMpsc;
    type Watch = TokioWatch;
    type Oneshot = MonoioOneshot;
    type Mutex<T: OptionalSend + 'static> = TokioMutex<T>;

    fn new(_threads: usize) -> Self {
        // Monoio is single-threaded, ignores threads parameter
        let rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
            .enable_all()
            .build()
            .expect("Failed to create Monoio runtime");
        MonoioRuntime { rt }
    }

    fn block_on<F, T>(&mut self, future: F) -> T
    where
        F: Future<Output = T>,
        T: OptionalSend,
    {
        self.rt.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use openraft_rt::testing::Suite;

    use super::*;

    #[test]
    fn test_monoio_rt() {
        MonoioRuntime::run(Suite::<MonoioRuntime>::test_all());
    }
}
