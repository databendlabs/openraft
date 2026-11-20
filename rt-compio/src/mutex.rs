use std::future::Future;

use openraft_rt::OptionalSend;
use openraft_rt::mutex;

pub struct FuturesMutex<T>(futures::lock::Mutex<T>);

impl<T> mutex::Mutex<T> for FuturesMutex<T>
where T: OptionalSend + 'static
{
    type Guard<'a> = futures::lock::MutexGuard<'a, T>;

    #[inline]
    fn new(value: T) -> Self {
        FuturesMutex(futures::lock::Mutex::new(value))
    }

    #[inline]
    fn lock(&self) -> impl Future<Output = Self::Guard<'_>> + OptionalSend {
        self.0.lock()
    }
}
