use std::fmt::Debug;
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

use crate::Instant;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::TokioInstant;

/// A trait defining interfaces with an asynchronous runtime.
///
/// The intention of this trait is to allow an application using this crate to bind an asynchronous
/// runtime that suits it the best.
///
/// ## Note
///
/// The default asynchronous runtime is `tokio`.
pub trait AsyncRuntime: Debug + Default + OptionalSend + OptionalSync + 'static {
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

    /// Abort the task associated with the supplied join handle.
    fn abort<T: OptionalSend + 'static>(join_handle: &Self::JoinHandle<T>);
}

/// `Tokio` is the default asynchronous executor.
#[derive(Debug, Default)]
pub struct TokioRuntime;

impl AsyncRuntime for TokioRuntime {
    type JoinError = tokio::task::JoinError;
    type JoinHandle<T: OptionalSend + 'static> = tokio::task::JoinHandle<T>;
    type Sleep = tokio::time::Sleep;
    type Instant = TokioInstant;
    type TimeoutError = tokio::time::error::Elapsed;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = tokio::time::Timeout<T>;

    #[inline]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        #[cfg(feature = "singlethreaded")]
        {
            tokio::task::spawn_local(future)
        }
        #[cfg(not(feature = "singlethreaded"))]
        {
            tokio::task::spawn(future)
        }
    }

    #[inline]
    fn sleep(duration: Duration) -> Self::Sleep {
        tokio::time::sleep(duration)
    }

    #[inline]
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep {
        tokio::time::sleep_until(deadline)
    }

    #[inline]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F> {
        tokio::time::timeout(duration, future)
    }

    #[inline]
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F> {
        tokio::time::timeout_at(deadline, future)
    }

    #[inline]
    fn is_panic(join_error: &Self::JoinError) -> bool {
        join_error.is_panic()
    }

    #[inline]
    fn abort<T: OptionalSend + 'static>(join_handle: &Self::JoinHandle<T>) {
        join_handle.abort();
    }
}
