//! Async mutex trait.

use std::future::Future;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::Arc;

use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;

/// Represents an implementation of an asynchronous Mutex.
pub trait Mutex<T>: OptionalSend + OptionalSync + 'static
where T: OptionalSend + 'static
{
    /// Handle to an acquired lock, should release it when dropped.
    type Guard<'a>: DerefMut<Target = T> + OptionalSend
    where Self: 'a;

    /// Creates a new lock.
    #[track_caller]
    fn new(value: T) -> Self;

    /// Locks this Mutex.
    ///
    /// The returned future resolves to a guard once the lock is acquired. The lock is exclusive:
    /// at most one guard of a mutex exists at a time, and other callers wait until it is dropped.
    ///
    /// The order in which waiting callers acquire the lock depends on the implementation. A mutex
    /// built on `tokio::sync::Mutex` serves them first-in-first-out, while one built on
    /// `futures::lock::Mutex` can give the lock to a caller that started waiting later. Code that
    /// is generic over the runtime must not rely on first-in-first-out order.
    #[track_caller]
    fn lock(&self) -> impl Future<Output = Self::Guard<'_>> + OptionalSend;

    /// Lock this mutex, returning a guard that owns the mutex via Arc.
    ///
    /// The guard can be moved, returned from functions, or stored in structs.
    /// This is provided as a default implementation for convenience.
    ///
    /// The lock is exclusive in the same way as with [`lock()`](Self::lock), and the order of
    /// waiting callers again depends on the implementation. The default implementation acquires
    /// the lock through `lock()`.
    #[must_use]
    fn lock_owned(self: Arc<Self>) -> impl Future<Output = OwnedGuard<Self, T>> + OptionalSend
    where Self: Sized {
        async move {
            let guard = self.lock().await;

            // SAFETY: `guard` borrows the mutex inside `self`. It is stored as `'static` next to
            // `self`, the `Arc` that keeps the mutex alive at the same address, and two rules keep
            // that sound. The guard is dropped before the `Arc`, so it can still reach the mutex
            // while it unlocks it. And the widened guard never leaves `OwnedGuard`: callers reach
            // only the guarded value, through `Deref` and `DerefMut`. See the fields of
            // `OwnedGuard`.
            //
            // Keeping the mutex alive says nothing about whether the guard may cross threads. That
            // is checked separately, by the `Send` and `Sync` impls of `OwnedGuard`.
            let guard_static = unsafe { std::mem::transmute::<Self::Guard<'_>, Self::Guard<'static>>(guard) };

            OwnedGuard {
                guard: guard_static,
                _mutex: self,
                _no_auto_send_sync: PhantomData,
            }
        }
    }
}

/// An owned guard that keeps the mutex alive via Arc.
///
/// This guard owns the lock through an `Arc<Mutex>`, allowing it to outlive
/// the original reference and be moved, returned from functions, or stored in structs.
///
/// Dropping the guard releases the lock, so a guard that is never bound protects nothing. It is
/// `#[must_use]`: with `unused_must_use` denied, discarding it does not compile.
///
/// ```compile_fail
/// #![deny(unused_must_use)]
///
/// use std::sync::Arc;
///
/// use openraft_rt::Mutex;
///
/// async fn discard<M>(mutex: Arc<M>)
/// where M: Mutex<u32> {
///     mutex.lock_owned().await;
/// }
/// ```
///
/// Binding the guard, or dropping it explicitly, compiles:
///
/// ```
/// #![deny(unused_must_use)]
///
/// use std::sync::Arc;
///
/// use openraft_rt::Mutex;
///
/// async fn bind<M>(mutex: Arc<M>)
/// where M: Mutex<u32> {
///     let mut guard = Arc::clone(&mutex).lock_owned().await;
///     *guard += 1;
///     drop(guard);
///
///     drop(mutex.lock_owned().await);
/// }
/// ```
#[must_use = "dropping the guard releases the lock"]
#[since(version = "0.10.2", change = "marked #[must_use]")]
#[since(version = "0.10.2", change = "`Send`/`Sync` check the guard at every lifetime")]
pub struct OwnedGuard<M, T>
where
    M: Mutex<T>,
    T: OptionalSend + 'static,
{
    // `guard` borrows the mutex in `_mutex`; its `'static` lifetime, given by
    // `Mutex::lock_owned()`, is not real. Two rules keep it sound.
    //
    // `guard` is declared before `_mutex`, so it is dropped first: the mutex is alive, at the
    // same address, until the guard's destructor returns. Do not reorder these fields.
    //
    // `guard` never leaves this struct: callers reach only its target, through `Deref` and
    // `DerefMut`.
    guard: M::Guard<'static>,
    _mutex: Arc<M>,

    /// Opts out of the automatic `Send` and `Sync`, which would check `M::Guard<'static>` instead
    /// of the guard of the real borrow.
    _no_auto_send_sync: PhantomData<*const ()>,
}

// Why `Send` and `Sync` check the guard at every lifetime
//
// `guard` is the `M::Guard<'a>` that `M::lock()` returned for a borrow of `_mutex`, stored as
// `M::Guard<'static>`. Automatic `Send` and `Sync` would check that stored type, but a guard
// type need not have the same `Send` or `Sync` at every lifetime. An adapter may, for example,
// use the guard
//
//     struct Guard<'a>(tokio::sync::MutexGuard<'a, u32>, PhantomData<Cell<()>>);
//
//     unsafe impl Sync for Guard<'static> {}
//
// whose marker field turns off the automatic `Sync`, which the impl then adds back only for
// `'static`. With automatic impls, the `OwnedGuard` of that adapter would be `Sync`, although
// the guard of the real borrow is not. `_no_auto_send_sync` turns the automatic impls off, and
// the impls below require the guard to be `Send` or `Sync` at every lifetime, `for<'a>`, and `M`
// to be `Send + Sync` for the `Arc`. Without `single-threaded`, `Mutex` already requires every
// guard to be `Send`, so there the difference is `Sync`; with `single-threaded`, it is both.
//
// Tokio's own owned guard does not need these bounds: its `OwnedMutexGuard` holds the
// `Arc<Mutex<T>>` itself and releases the lock in its own destructor, so it never widens the
// lifetime of a borrowed guard, and it can state its `Send` and `Sync` in terms of `T`.
// `OwnedGuard` works with the guard of any adapter, so it can rely only on what that guard
// promises.

// SAFETY: `guard` is an `M::Guard<'a>` of a borrow of `_mutex`, so it may be sent to another thread
// only when the guard is `Send` at every lifetime. `Arc<M>` is `Send` when `M` is `Send + Sync`.
unsafe impl<M, T> Send for OwnedGuard<M, T>
where
    M: Mutex<T> + Send + Sync,
    T: OptionalSend + 'static,
    for<'a> M::Guard<'a>: Send,
{
}

// SAFETY: As for `Send`: the guard must be `Sync` at every lifetime, and `Arc<M>` is `Sync` when
// `M` is `Send + Sync`.
unsafe impl<M, T> Sync for OwnedGuard<M, T>
where
    M: Mutex<T> + Send + Sync,
    T: OptionalSend + 'static,
    for<'a> M::Guard<'a>: Sync,
{
}

impl<M, T> Deref for OwnedGuard<M, T>
where
    M: Mutex<T>,
    T: OptionalSend + 'static,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<M, T> DerefMut for OwnedGuard<M, T>
where
    M: Mutex<T>,
    T: OptionalSend + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

#[cfg(test)]
mod tests {
    use std::future::ready;
    use std::sync::Weak;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use futures_util::FutureExt;

    use super::*;

    /// A mutex whose guard records how many `Arc`s still own the mutex when the guard is dropped.
    struct CountingMutex {
        this: Weak<CountingMutex>,
        owners_at_guard_drop: AtomicUsize,
    }

    /// A guard over a value of its own: it checks the drop order, not mutual exclusion.
    struct CountingGuard<'a> {
        mutex: &'a CountingMutex,
        value: u32,
    }

    impl Deref for CountingGuard<'_> {
        type Target = u32;

        fn deref(&self) -> &u32 {
            &self.value
        }
    }

    impl DerefMut for CountingGuard<'_> {
        fn deref_mut(&mut self) -> &mut u32 {
            &mut self.value
        }
    }

    impl Drop for CountingGuard<'_> {
        fn drop(&mut self) {
            let owners = self.mutex.this.strong_count();
            self.mutex.owners_at_guard_drop.store(owners, Ordering::Relaxed);
        }
    }

    impl Mutex<u32> for CountingMutex {
        type Guard<'a> = CountingGuard<'a>;

        fn new(_value: u32) -> Self {
            Self {
                this: Weak::new(),
                owners_at_guard_drop: AtomicUsize::new(0),
            }
        }

        fn lock(&self) -> impl Future<Output = CountingGuard<'_>> + OptionalSend {
            ready(CountingGuard { mutex: self, value: 0 })
        }
    }

    #[test]
    fn test_owned_guard_drops_the_guard_before_its_arc() {
        let mutex = Arc::new_cyclic(|this| CountingMutex {
            this: this.clone(),
            owners_at_guard_drop: AtomicUsize::new(0),
        });
        let other_owner = Arc::clone(&mutex);

        let guard = mutex.lock_owned().now_or_never().unwrap();
        drop(guard);

        assert_eq!(
            2,
            other_owner.owners_at_guard_drop.load(Ordering::Relaxed),
            "the OwnedGuard should still own its Arc while the guard is dropped"
        );
        assert_eq!(
            1,
            Arc::strong_count(&other_owner),
            "the OwnedGuard should then release its Arc"
        );
    }

    #[cfg(not(feature = "single-threaded"))]
    #[test]
    fn test_owned_guard_is_send_and_sync() {
        fn assert_send_sync<X>()
        where X: Send + Sync {
        }
        assert_send_sync::<OwnedGuard<CountingMutex, u32>>();
    }

    /// Compiles only if the `OwnedGuard` of every `Mutex` is `Send`, as without `single-threaded`
    /// the trait requires each guard to be `Send`.
    #[cfg(not(feature = "single-threaded"))]
    #[allow(dead_code)]
    fn owned_guard_of_any_mutex_is_send<M, T>()
    where
        M: Mutex<T>,
        T: OptionalSend + 'static,
    {
        fn assert_send<X>()
        where X: Send {
        }
        assert_send::<OwnedGuard<M, T>>();
    }
}
