use std::cell::UnsafeCell;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::TypeConfigExt;
use crate::RaftTypeConfig;

/// A simple async mutex implementation that uses oneshot channels to notify the next waiting task.
///
/// Openraft use async mutex in non-performance critical path,
/// so it's ok to use this simple implementation.
///
/// Since oneshot channel is already required by AsyncRuntime implementation,
/// there is no need for the application to implement Mutex.
pub(crate) struct Mutex<C, T>
where C: RaftTypeConfig
{
    /// The current lock holder.
    ///
    /// When the acquired `MutexGuard` is dropped, it will notify the next waiting task via this
    /// oneshot channel.
    holder: std::sync::Mutex<Option<OneshotReceiverOf<C, ()>>>,

    /// The value protected by the mutex.
    value: UnsafeCell<T>,
}

impl<C, T> Mutex<C, T>
where C: RaftTypeConfig
{
    pub fn new(value: T) -> Self {
        Self {
            holder: std::sync::Mutex::new(None),
            value: UnsafeCell::new(value),
        }
    }

    pub async fn lock(&self) -> MutexGuard<'_, C, T> {
        // Every lock() call puts a oneshot receiver into the holder
        // and takes out the existing one.
        // If the existing one is Some(rx),
        // it means the lock is already held by another task.
        // In this case, the current task should wait for the lock to be released.
        //
        // Such approach forms a queue in which every task waits for the previous one.

        let (tx, rx) = C::oneshot();
        let current_rx = {
            let mut l = self.holder.lock().unwrap();
            l.replace(rx)
        };

        if let Some(rx) = current_rx {
            rx.await;
        }

        MutexGuard { guard: tx, lock: self }
    }

    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

/// The guard of the mutex.
pub(crate) struct MutexGuard<'a, C, T>
where C: RaftTypeConfig
{
    guard: OneshotSenderOf<C, ()>,
    lock: &'a Mutex<C, T>,
}

impl<'a, C, T> Deref for MutexGuard<'a, C, T>
where C: RaftTypeConfig
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.value.get() }
    }
}

impl<'a, C, T> DerefMut for MutexGuard<'a, C, T>
where C: RaftTypeConfig
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.value.get() }
    }
}

/// T must be `Send` to make Mutex `Send`
unsafe impl<C: RaftTypeConfig, T> Send for Mutex<C, T> where T: Send {}

/// To allow multiple threads to access T through a `&Mutex`, T must be `Send`.
/// T being `Sync` is not enough: because the caller acquires the ownership through `Mutex::lock()`.
unsafe impl<C: RaftTypeConfig, T> Sync for Mutex<C, T> where T: Send {}

// TODO: doc
unsafe impl<C: RaftTypeConfig, T> Sync for MutexGuard<'_, C, T> where T: Send + Sync {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::engine::testing::UTConfig;

    #[test]
    fn bounds() {
        fn check_send<T: Send>() {}
        fn check_unpin<T: Unpin>() {}
        // This has to take a value, since the async fn's return type is unnameable.
        fn check_send_sync_val<T: Send + Sync>(_t: T) {}
        fn check_send_sync<T: Send + Sync>() {}

        check_send::<MutexGuard<'_, UTConfig, u32>>();
        check_unpin::<Mutex<UTConfig, u32>>();
        check_send_sync::<Mutex<UTConfig, u32>>();

        let mutex = Mutex::<UTConfig, _>::new(1);
        check_send_sync_val(mutex.lock());
    }

    #[test]
    fn test_mutex() {
        let mutex = Arc::new(Mutex::<UTConfig, u64>::new(0));

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(8)
            .enable_all()
            .build()
            .expect("Failed building the Runtime");

        let big_prime_num = 1_000_000_009;
        let n = 100_000;
        let n_task = 10;
        let mut tasks = vec![];

        for i in 0..n_task {
            let mutex = mutex.clone();
            let h = rt.spawn(async move {
                for k in 0..n {
                    {
                        let mut guard = mutex.lock().await;
                        *guard = (*guard + k) % big_prime_num;
                    }
                }
            });

            tasks.push(h);
        }

        let got = rt.block_on(async {
            for t in tasks {
                let _ = t.await;
            }
            *mutex.lock().await
        });

        println!("got: {}", got);
        assert_eq!(got, n_task * n * (n - 1) / 2 % big_prime_num);
    }
}
