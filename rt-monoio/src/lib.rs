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

/// [`AsyncRuntime`] implementation for Monoio.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MonoioRuntime;

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
}

#[cfg(test)]
mod tests {
    use openraft_rt::testing::Suite;

    use super::*;

    #[test]
    fn test_monoio_rt() {
        let mut rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new().enable_all().build().unwrap();
        rt.block_on(Suite::<MonoioRuntime>::test_all());
    }
}
