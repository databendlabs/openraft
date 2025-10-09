//! `async` runtime interface.
//!
//! `async` runtime is an abstraction over different asynchronous runtimes, such as `tokio`,
//! `async-std`, etc.

pub(crate) mod tokio_impls {
    #![cfg(feature = "tokio-rt")]

    mod tokio_runtime;
    pub use tokio_runtime::TokioRuntime;
}
pub mod mpsc;
pub mod mpsc_unbounded;
pub mod mutex;
pub mod oneshot;
pub mod watch;

use std::fmt::Debug;
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

pub use mpsc::Mpsc;
pub use mpsc::MpscReceiver;
pub use mpsc::MpscSender;
pub use mpsc::MpscWeakSender;
pub use mpsc_unbounded::MpscUnbounded;
pub use mpsc_unbounded::MpscUnboundedReceiver;
pub use mpsc_unbounded::MpscUnboundedSender;
pub use mpsc_unbounded::MpscUnboundedWeakSender;
pub use mpsc_unbounded::SendError;
pub use mpsc_unbounded::TryRecvError;
pub use mutex::Mutex;
pub use oneshot::Oneshot;
pub use oneshot::OneshotSender;
pub use watch::Watch;

use crate::Instant;
use crate::OptionalSend;
use crate::OptionalSync;

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
pub trait AsyncRuntime: Debug + Default + PartialEq + Eq + OptionalSend + OptionalSync + 'static {
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
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static;

    /// Wait until `duration` has elapsed.
    fn sleep(duration: Duration) -> Self::Sleep;

    /// Wait until `deadline` is reached.
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep;

    /// Require a [`Future`] to complete before the specified duration has elapsed.
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F>;

    /// Require a [`Future`] to complete before the specified instant in time.
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F>;

    /// Check if the [`Self::JoinError`] is `panic`.
    fn is_panic(join_error: &Self::JoinError) -> bool;

    /// Get the random number generator to use for generating random numbers.
    ///
    /// # Note
    ///
    /// This is a per-thread instance, which cannot be shared across threads or
    /// sent to another thread.
    fn thread_rng() -> Self::ThreadLocalRng;

    /// The bounded MPSC channel implementation.
    type Mpsc: Mpsc;

    /// The unbounded MPSC channel implementation.
    type MpscUnbounded: MpscUnbounded;

    /// The watch channel implementation.
    type Watch: Watch;

    /// The oneshot channel implementation.
    type Oneshot: Oneshot;

    /// The async mutex implementation.
    type Mutex<T: OptionalSend + 'static>: Mutex<T>;
}
