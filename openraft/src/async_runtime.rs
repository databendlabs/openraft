use std::fmt::Debug;
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;
use std::time::Instant;

/// A trait defining interfaces with an asynchronous runtime.
///
/// The intention of this trait is to allow an application this crate to choose an asynchronous
/// runtime that best suits it.
///
/// ## Note
///
/// The default asynchronous runtime is `tokio`.
pub trait AsyncRuntime: Send + Sync + 'static {
    /// The type that [`Self::JoinHandle`] returns on failure.
    type JoinError: Debug + Display + Send;

    /// The return type of [`Self::spawn`].
    type JoinHandle<T: Send + 'static>: Future<Output = Result<T, Self::JoinError>> + Send + Sync + Unpin;

    /// The type that enables the user to sleep.
    type Sleep: Future<Output = ()> + Send + Sync;

    /// The timeout error type.
    type TimeoutError: Debug + Display + Send;

    /// The timeout type used by [`Self::timeout`].
    type Timeout<R, T: Future<Output = R> + Send>: Future<Output = Result<R, Self::TimeoutError>> + Send;

    /// Spawn a new task.
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + Send + 'static,
        T::Output: Send + 'static;

    /// Waits until `duration` has elapsed.
    fn sleep(duration: Duration) -> Self::Sleep;

    /// Waits until `deadline` is reached.
    fn sleep_until(deadline: Instant) -> Self::Sleep;

    /// Returns the current instant.
    fn now() -> Instant;

    /// Requires a [`Future`] to complete before the specified duration has elapsed.
    fn timeout<R, F: Future<Output = R> + Send>(duration: Duration, future: F) -> Self::Timeout<R, F>;

    /// Requires a [`Future`] to complete before the specified instant in time.
    fn timeout_at<R, F: Future<Output = R> + Send>(deadline: Instant, future: F) -> Self::Timeout<R, F>;

    /// Checks if the [`Self::JoinError`] is `panic`.
    fn is_panic(join_error: &Self::JoinError) -> bool;

    /// Abort the task associated with the supplied join handle.
    fn abort<T: Send + 'static>(join_handle: &Self::JoinHandle<T>);
}

/// `Tokio` is the default asynchronous executor.
pub struct Tokio;

impl AsyncRuntime for Tokio {
    type JoinError = tokio::task::JoinError;
    type JoinHandle<T: Send + 'static> = tokio::task::JoinHandle<T>;
    type Sleep = tokio::time::Sleep;
    type TimeoutError = tokio::time::error::Elapsed;
    type Timeout<R, T: Future<Output = R> + Send> = tokio::time::Timeout<T>;

    #[inline]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + Send + 'static,
        T::Output: Send + 'static,
    {
        tokio::task::spawn(future)
    }

    #[inline]
    fn sleep(duration: Duration) -> Self::Sleep {
        tokio::time::sleep(duration)
    }

    #[inline]
    fn sleep_until(deadline: Instant) -> Self::Sleep {
        tokio::time::sleep_until(deadline.into())
    }

    #[inline]
    fn now() -> Instant {
        tokio::time::Instant::now().into()
    }

    #[inline]
    fn timeout<R, F: Future<Output = R> + Send>(duration: Duration, future: F) -> Self::Timeout<R, F> {
        tokio::time::timeout(duration, future)
    }

    #[inline]
    fn timeout_at<R, F: Future<Output = R> + Send>(deadline: Instant, future: F) -> Self::Timeout<R, F> {
        tokio::time::timeout_at(deadline.into(), future)
    }

    #[inline]
    fn is_panic(join_error: &Self::JoinError) -> bool {
        join_error.is_panic()
    }

    #[inline]
    fn abort<T: Send + 'static>(join_handle: &Self::JoinHandle<T>) {
        join_handle.abort();
    }
}
