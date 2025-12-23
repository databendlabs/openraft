use std::future::Future;

use crate::OptionalSend;
use crate::async_runtime::mutex;

pub type TokioMutex<T> = tokio::sync::Mutex<T>;

impl<T> mutex::Mutex<T> for TokioMutex<T>
where T: OptionalSend + 'static
{
    type Guard<'a> = tokio::sync::MutexGuard<'a, T>;

    fn new(value: T) -> Self {
        TokioMutex::new(value)
    }

    fn lock(&self) -> impl Future<Output = Self::Guard<'_>> + OptionalSend {
        self.lock()
    }
}
