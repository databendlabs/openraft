use std::future::Future;

use openraft_rt::OptionalSend;
use openraft_rt::mutex;

/// Wrapper around `tokio::sync::Mutex` to implement the `Mutex` trait.
pub struct TokioMutex<T>(tokio::sync::Mutex<T>);

impl<T> mutex::Mutex<T> for TokioMutex<T>
where T: OptionalSend + 'static
{
    type Guard<'a> = tokio::sync::MutexGuard<'a, T>;

    fn new(value: T) -> Self {
        TokioMutex(tokio::sync::Mutex::new(value))
    }

    fn lock(&self) -> impl Future<Output = Self::Guard<'_>> + OptionalSend {
        self.0.lock()
    }
}
