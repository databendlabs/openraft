//! Type-map for storing user-defined extension data.
//!
//! [`Extensions`] allows external crates to store arbitrary types associated with a Raft instance.
//! Each type can have at most one value stored, keyed by its [`TypeId`].
//!
//! # Example
//!
//! ```ignore
//! use std::sync::atomic::{AtomicU64, Ordering};
//! use std::sync::Arc;
//!
//! // Use Arc for shared mutable state
//! #[derive(Clone, Default)]
//! pub struct MyCounter(Arc<AtomicU64>);
//!
//! // Get a clone (auto-inserts default if not present)
//! let counter = raft.extensions().get::<MyCounter>();
//! counter.0.fetch_add(1, Ordering::Relaxed);
//! ```

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;

use openraft_macros::since;

use crate::base::BoxAny;
use crate::base::OptionalSend;

/// The value of one type, `None` until it has one.
///
/// Each type has its own lock, so that the callbacks of one type do not block access to the
/// others.
type Slot = Arc<Mutex<Option<BoxAny>>>;

/// A type-map for storing user-defined extension data.
///
/// Values are stored by their [`TypeId`], so each type can have at most one value.
/// Access is thread-safe.
///
/// Values must implement [`Clone`] and [`Default`]. When retrieved via [`get()`](Self::get),
/// a clone is returned. If no value exists, `T::default()` is stored before cloning.
///
/// `T::default()` and `T::clone()` run while only the value of `T` is locked. They can access
/// the values of other types, but must not access `T` itself, directly or through the callbacks
/// of other types: that would deadlock. A panic in them leaves the values of other types usable.
#[since(version = "0.10.2", change = "`get()` callbacks run outside the map-wide lock")]
pub struct Extensions {
    map: Mutex<HashMap<TypeId, Slot>>,
}

impl Extensions {
    /// Get a clone of the value of type `T`.
    ///
    /// If no value exists, `T::default()` is inserted and a clone is returned. Concurrent first
    /// lookups of `T` store a single default.
    pub fn get<T>(&self) -> T
    where T: OptionalSend + Clone + Default + 'static {
        let type_id = TypeId::of::<T>();
        loop {
            let slot = lock(&self.map).entry(type_id).or_default().clone();
            let mut stored = lock(&slot);

            // A concurrent `remove()` may have detached this slot after the lookup above.
            if !self.is_current(type_id, &slot) {
                continue;
            }

            let value = stored.get_or_insert_with(|| Box::new(T::default()));
            return value.downcast_ref::<T>().expect("extension type must match its TypeId").clone();
        }
    }

    /// Check if a value of type `T` exists.
    pub fn contains<T>(&self) -> bool
    where T: 'static {
        let slot = lock(&self.map).get(&TypeId::of::<T>()).cloned();
        slot.is_some_and(|slot| lock(&slot).is_some())
    }

    /// Remove and return a value of type `T`.
    pub fn remove<T>(&self) -> Option<T>
    where T: 'static {
        let slot = lock(&self.map).remove(&TypeId::of::<T>())?;
        let value = lock(&slot).take()?;
        value.downcast().ok().map(|b| *b)
    }

    /// Whether `slot` is still the slot of `type_id` in the map.
    fn is_current(&self, type_id: TypeId, slot: &Slot) -> bool {
        lock(&self.map).get(&type_id).is_some_and(|current| Arc::ptr_eq(current, slot))
    }
}

/// Lock `mutex`, also after a panic in an extension callback poisoned it.
///
/// The data stays consistent: `T::default()` runs before its value is stored, and `T::clone()`
/// only reads it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Default for Extensions {
    fn default() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.map.lock().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("Extensions").field("count", &count).finish()
    }
}

#[cfg(test)]
mod extensions_test;
