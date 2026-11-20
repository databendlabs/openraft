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
//! 2. With the `single-threaded` feature enabled, the handle type [`Raft`] will no longer be
//!    [`Send`] and [`Sync`].
//! 3. Even though this crate allows you to use Monoio, it still uses some primitives from Tokio
//!    1. `Watch`: Monoio (or `local_sync`) does not have a watch channel.
//!    2. `Mutex`: Monoio does not provide a Mutex implementation.
//!    3. `Mpsc`: Monoio has no channel, and the bounded channel of `local_sync` has no weak sender,
//!       which [`MpscSender::downgrade`](openraft_rt::MpscSender::downgrade) needs.
//!
//!    `Oneshot` uses the oneshot channel of `local_sync`.
//!
//! [`Raft`]: https://docs.rs/openraft/latest/openraft/raft/struct.Raft.html

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
///
/// # Panics in spawned tasks
///
/// Monoio does not catch a panic in a task spawned with [`AsyncRuntime::spawn`]. The panic unwinds
/// out of the `block_on` call that drives the runtime, before any task that awaits the spawned
/// task's [`JoinHandle`](AsyncRuntime::JoinHandle) gets a result. Joining therefore never fails,
/// and [`JoinError`](AsyncRuntime::JoinError) is [`Infallible`](std::convert::Infallible).
///
/// As a result, Openraft cannot report a panic in its core task as
/// `openraft::errors::Fatal::Panicked`. The panic ends the `block_on` call instead, and no Openraft
/// caller on this runtime gets an error.
pub struct MonoioRuntime {
    rt: InnerRuntime,
}

impl std::fmt::Debug for MonoioRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonoioRuntime").finish()
    }
}

impl AsyncRuntime for MonoioRuntime {
    /// Joining a task never fails: Monoio does not catch a panic in a spawned task, and the panic
    /// unwinds out of `block_on` instead of reaching the [`JoinHandle`](AsyncRuntime::JoinHandle).
    /// See [`MonoioRuntime`].
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
    where F: Future<Output = T> {
        self.rt.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use openraft_rt::testing::Suite;

    use super::*;

    #[test]
    fn test_monoio_rt() {
        Suite::<MonoioRuntime>::test_all();
    }

    #[test]
    fn test_spawned_task_panic_unwinds_out_of_block_on() {
        struct ChildPanic;

        // The panic ends the thread that drives the runtime, so the runtime gets a thread of its
        // own, and the test reads the panic from that thread's join.
        let joined = std::thread::spawn(|| {
            let mut rt = MonoioRuntime::new(1);
            rt.block_on(async {
                let child = MonoioRuntime::spawn(async { std::panic::panic_any(ChildPanic) });
                // Bound the wait, so that a join that never returns fails the test instead of
                // hanging it.
                let _ = MonoioRuntime::timeout(Duration::from_secs(10), child).await;
            })
        })
        .join();

        let payload = joined.expect_err("block_on returned after the spawned task panicked");
        assert!(payload.is::<ChildPanic>());
    }
}
