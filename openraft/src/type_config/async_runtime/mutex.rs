//! Async mutex trait.

use std::future::Future;
use std::ops::DerefMut;

use crate::OptionalSend;
use crate::OptionalSync;

/// Represents an implementation of an asynchronous Mutex.
pub trait Mutex<T: OptionalSend + 'static>: OptionalSend + OptionalSync {
    /// Handle to an acquired lock, should release it when dropped.
    type Guard<'a>: DerefMut<Target = T> + OptionalSend
    where Self: 'a;

    /// Creates a new lock.
    fn new(value: T) -> Self;

    /// Locks this Mutex.
    fn lock(&self) -> impl Future<Output = Self::Guard<'_>> + OptionalSend;
}
