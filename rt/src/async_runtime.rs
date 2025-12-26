//! `async` runtime interface.
//!
//! `async` runtime is an abstraction over different asynchronous runtimes, such as `tokio`,
//! `async-std`, etc.

use std::fmt::Debug;
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

use crate::Instant;
use crate::Mpsc;
use crate::Mutex;
use crate::Oneshot;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::Watch;

/// A trait defining interfaces with an asynchronous runtime.
///
/// The intention of this trait is to allow an application using this crate to bind an asynchronous
/// runtime that suits it the best.
///
/// Some additional related functions are also exposed by this trait.
///
/// ## Note
///
/// The default asynchronous runtime is `tokio`.
pub trait AsyncRuntime: Debug + OptionalSend + OptionalSync + 'static {
    /// The error type of [`Self::JoinHandle`].
    type JoinError: Debug + Display + OptionalSend;

    /// The return type of [`Self::spawn`].
    type JoinHandle<T: OptionalSend + 'static>: Future<Output = Result<T, Self::JoinError>>
        + OptionalSend
        + OptionalSync
        + Unpin;

    /// The type that enables the user to sleep in an asynchronous runtime.
    type Sleep: Future<Output = ()> + OptionalSend + OptionalSync;

    /// A measurement of a monotonically non-decreasing clock.
    type Instant: Instant;

    /// The timeout error type.
    type TimeoutError: Debug + Display + OptionalSend;

    /// The timeout type used by [`Self::timeout`] and [`Self::timeout_at`] that enables the user
    /// to await the outcome of a [`Future`].
    type Timeout<R, T: Future<Output = R> + OptionalSend>: Future<Output = Result<R, Self::TimeoutError>> + OptionalSend;

    /// Type of thread-local random number generator.
    type ThreadLocalRng: rand::Rng;

    /// Spawn a new task.
    #[track_caller]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static;

    /// Wait until `duration` has elapsed.
    #[track_caller]
    fn sleep(duration: Duration) -> Self::Sleep;

    /// Wait until `deadline` is reached.
    #[track_caller]
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep;

    /// Require a [`Future`] to complete before the specified duration has elapsed.
    #[track_caller]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F>;

    /// Require a [`Future`] to complete before the specified instant in time.
    #[track_caller]
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F>;

    /// Check if the [`Self::JoinError`] is `panic`.
    #[track_caller]
    fn is_panic(join_error: &Self::JoinError) -> bool;

    /// Get the random number generator to use for generating random numbers.
    ///
    /// # Note
    ///
    /// This is a per-thread instance, which cannot be shared across threads or
    /// sent to another thread.
    #[track_caller]
    fn thread_rng() -> Self::ThreadLocalRng;

    /// The bounded MPSC channel implementation.
    type Mpsc: Mpsc;

    /// The watch channel implementation.
    type Watch: Watch;

    /// The oneshot channel implementation.
    type Oneshot: Oneshot;

    /// The async mutex implementation.
    type Mutex<T: OptionalSend + 'static>: Mutex<T>;

    /// Create a new runtime instance for testing purposes.
    ///
    /// **Note**: This method is primarily intended for testing and is not used by Openraft
    /// internally. In production applications, the runtime should be created and managed
    /// by the application itself, with Openraft running within that runtime.
    ///
    /// # Arguments
    ///
    /// * `threads` - Number of worker threads. Multi-threaded runtimes (like Tokio) will use this
    ///   value; single-threaded runtimes (like Monoio, Compio) may ignore it.
    fn new(threads: usize) -> Self;

    /// Run a future to completion on this runtime.
    ///
    /// This runs synchronously on the current thread, so `Send` is not required.
    fn block_on<F, T>(&mut self, future: F) -> T
    where
        F: Future<Output = T>,
        T: OptionalSend;

    /// Convenience method: create a runtime and run the future to completion.
    ///
    /// Creates a runtime with default configuration (8 threads) and runs the future.
    /// For simple cases where you don't need to reuse the runtime.
    /// If you need to run multiple futures, consider using [`Self::new`] and
    /// [`Self::block_on`] directly.
    ///
    /// This runs synchronously on the current thread, so `Send` is not required.
    fn run<F, T>(future: F) -> T
    where
        Self: Sized,
        F: Future<Output = T>,
        T: OptionalSend,
    {
        Self::new(8).block_on(future)
    }
}
