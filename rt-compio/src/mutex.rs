use std::future::Future;

use openraft::type_config::async_runtime::mutex;
use openraft::OptionalSend;

pub struct TokioMutex<T>(tokio::sync::Mutex<T>);

impl<T> mutex::Mutex<T> for TokioMutex<T>
where T: OptionalSend + 'static
{
    type Guard<'a> = tokio::sync::MutexGuard<'a, T>;

    #[inline]
    fn new(value: T) -> Self {
        TokioMutex(tokio::sync::Mutex::new(value))
    }

    #[inline]
    fn lock(&self) -> impl Future<Output = Self::Guard<'_>> + OptionalSend {
        self.0.lock()
    }
}
