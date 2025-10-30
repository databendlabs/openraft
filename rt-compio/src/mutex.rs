use std::future::Future;

use openraft::type_config::async_runtime::mutex;
use openraft::OptionalSend;

pub struct FlumeMutex<T>(futures::lock::Mutex<T>);

impl<T> mutex::Mutex<T> for FlumeMutex<T>
where T: OptionalSend + 'static
{
    type Guard<'a> = futures::lock::MutexGuard<'a, T>;

    #[inline]
    fn new(value: T) -> Self {
        FlumeMutex(futures::lock::Mutex::new(value))
    }

    #[inline]
    fn lock(&self) -> impl Future<Output = Self::Guard<'_>> + OptionalSend {
        self.0.lock()
    }
}
