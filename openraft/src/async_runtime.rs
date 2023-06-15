use std::future::Future;

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
    type JoinError: std::fmt::Display;

    /// The return type of [`Self::spawn`].
    type JoinHandle<T: Send + 'static>: Future<Output = Result<T, Self::JoinError>> + Send + Sync;

    /// Spawn a new task.
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + Send + 'static,
        T::Output: Send + 'static;

    /// Abort the task associated with the supplied join handle.
    fn abort<T: Send + 'static>(join_handle: &Self::JoinHandle<T>);
}

/// `Tokio` is the default asynchronous executor.
pub struct Tokio;

impl AsyncRuntime for Tokio {
    type JoinError = tokio::task::JoinError;
    type JoinHandle<T: Send + 'static> = tokio::task::JoinHandle<T>;

    #[inline]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + Send + 'static,
        T::Output: Send + 'static,
    {
        tokio::task::spawn(future)
    }

    #[inline]
    fn abort<T: Send + 'static>(join_handle: &Self::JoinHandle<T>) {
        join_handle.abort();
    }
}
