//! Async mutex trait.

use std::future::Future;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::Arc;

use crate::OptionalSend;
use crate::OptionalSync;

/// Represents an implementation of an asynchronous Mutex.
pub trait Mutex<T: OptionalSend + 'static>: OptionalSend + OptionalSync + 'static {
    /// Handle to an acquired lock, should release it when dropped.
    type Guard<'a>: DerefMut<Target = T> + OptionalSend
    where Self: 'a;

    /// Creates a new lock.
    #[track_caller]
    fn new(value: T) -> Self;

    /// Locks this Mutex.
    #[track_caller]
    fn lock(&self) -> impl Future<Output = Self::Guard<'_>> + OptionalSend;

    /// Lock this mutex, returning a guard that owns the mutex via Arc.
    ///
    /// The guard can be moved, returned from functions, or stored in structs.
    /// This is provided as a default implementation for convenience.
    #[must_use]
    fn lock_owned(self: Arc<Self>) -> impl Future<Output = OwnedGuard<Self, T>> + OptionalSend
    where Self: Sized {
        async move {
            let guard = self.lock().await;

            // SAFETY: OwnedGuard holds the Arc, ensuring mutex outlives the guard.
            // We transmute the guard to 'static lifetime because its lifetime is
            // now tied to the Arc in OwnedGuard, not the original borrow.
            let guard_static = unsafe { std::mem::transmute::<Self::Guard<'_>, Self::Guard<'static>>(guard) };

            OwnedGuard {
                guard: guard_static,
                mutex: self,
            }
        }
    }
}

/// An owned guard that keeps the mutex alive via Arc.
///
/// This guard owns the lock through an `Arc<Mutex>`, allowing it to outlive
/// the original reference and be moved, returned from functions, or stored in structs.
pub struct OwnedGuard<M: Mutex<T>, T>
where T: OptionalSend + 'static
{
    guard: M::Guard<'static>,
    #[allow(dead_code)]
    mutex: Arc<M>,
}

impl<M: Mutex<T>, T> Deref for OwnedGuard<M, T>
where T: OptionalSend + 'static
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<M: Mutex<T>, T> DerefMut for OwnedGuard<M, T>
where T: OptionalSend + 'static
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}
