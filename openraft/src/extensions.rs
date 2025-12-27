//! Type-map for storing user-defined extension data.
//!
//! [`Extensions`] allows external crates to store arbitrary types associated with a Raft instance.
//! Each type can have at most one value stored, keyed by its [`TypeId`].
//!
//! This follows the `http::Extensions` pattern.
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
//! // Insert at startup
//! raft.extensions().insert(MyCounter::default());
//!
//! // Get a clone and use it
//! if let Some(counter) = raft.extensions().get::<MyCounter>() {
//!     counter.0.fetch_add(1, Ordering::Relaxed);
//! }
//! ```

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::base::BoxAny;
use crate::base::OptionalSend;

/// A type-map for storing user-defined extension data.
///
/// Values are stored by their [`TypeId`], so each type can have at most one value.
/// Access is thread-safe via internal [`Mutex`].
///
/// Values must implement [`Clone`] - when retrieved via [`get()`](Self::get),
/// a clone is returned to avoid holding locks.
pub struct Extensions {
    map: Mutex<HashMap<TypeId, BoxAny>>,
}

impl Extensions {
    /// Create a new empty Extensions.
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    /// Insert a value of type `T`.
    ///
    /// Returns the previous value if one existed.
    pub fn insert<T>(&self, val: T) -> Option<T>
    where T: OptionalSend + Clone + 'static {
        let mut map = self.map.lock().unwrap();
        map.insert(TypeId::of::<T>(), Box::new(val)).and_then(|boxed| boxed.downcast().ok().map(|b| *b))
    }

    /// Get a clone of the value of type `T`.
    ///
    /// Returns [`None`] if no value of type `T` has been inserted.
    pub fn get<T>(&self) -> Option<T>
    where T: Clone + 'static {
        let map = self.map.lock().unwrap();
        map.get(&TypeId::of::<T>()).and_then(|b| b.downcast_ref::<T>()).cloned()
    }

    /// Get a clone of the value of type `T`, or insert a default value if none exists.
    ///
    /// If no value of type `T` has been inserted, the provided closure is called
    /// to create a default value, which is then inserted and returned.
    pub fn get_or_insert_with<T>(&self, f: impl FnOnce() -> T) -> T
    where T: OptionalSend + Clone + 'static {
        let mut map = self.map.lock().unwrap();
        let type_id = TypeId::of::<T>();

        if let Some(val) = map.get(&type_id).and_then(|b| b.downcast_ref::<T>()) {
            return val.clone();
        }

        let val = f();
        map.insert(type_id, Box::new(val.clone()));
        val
    }

    /// Get a clone of the value of type `T`, or insert `T::default()` if none exists.
    ///
    /// This is a convenience method equivalent to `get_or_insert_with(T::default)`.
    pub fn get_or_default<T>(&self) -> T
    where T: OptionalSend + Clone + Default + 'static {
        self.get_or_insert_with(T::default)
    }

    /// Check if a value of type `T` exists.
    pub fn contains<T>(&self) -> bool
    where T: 'static {
        let map = self.map.lock().unwrap();
        map.contains_key(&TypeId::of::<T>())
    }

    /// Remove and return a value of type `T`.
    pub fn remove<T>(&self) -> Option<T>
    where T: 'static {
        let mut map = self.map.lock().unwrap();
        map.remove(&TypeId::of::<T>()).and_then(|boxed| boxed.downcast().ok().map(|b| *b))
    }
}

impl Default for Extensions {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.map.lock().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("Extensions").field("count", &count).finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;

    use super::*;

    #[test]
    fn test_insert_and_get() {
        let ext = Extensions::new();

        ext.insert(42u32);
        ext.insert("hello".to_string());

        assert_eq!(ext.get::<u32>().unwrap(), 42);
        assert_eq!(ext.get::<String>().unwrap(), "hello");
        assert!(ext.get::<i64>().is_none());
    }

    #[test]
    fn test_insert_replaces() {
        let ext = Extensions::new();

        assert!(ext.insert(1u32).is_none());
        assert_eq!(ext.insert(2u32), Some(1));
        assert_eq!(ext.get::<u32>().unwrap(), 2);
    }

    #[test]
    fn test_shared_state_with_arc() {
        let ext = Extensions::new();

        // Use Arc for shared mutable state
        #[derive(Clone, Default)]
        struct Counter(Arc<AtomicU64>);

        ext.insert(Counter::default());

        // Get clone and mutate
        let counter1 = ext.get::<Counter>().unwrap();
        counter1.0.fetch_add(1, Ordering::Relaxed);
        counter1.0.fetch_add(1, Ordering::Relaxed);

        // Get another clone - shares the same Arc
        let counter2 = ext.get::<Counter>().unwrap();
        assert_eq!(counter2.0.load(Ordering::Relaxed), 2);

        // More increments
        counter2.0.fetch_add(1, Ordering::Relaxed);
        assert_eq!(counter1.0.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn test_remove() {
        let ext = Extensions::new();

        ext.insert(42u32);
        assert!(ext.contains::<u32>());

        assert_eq!(ext.remove::<u32>(), Some(42));
        assert!(!ext.contains::<u32>());
        assert!(ext.remove::<u32>().is_none());
    }

    #[test]
    fn test_get_or_insert_with() {
        let ext = Extensions::new();

        // First call creates the value
        let val = ext.get_or_insert_with(|| 42u32);
        assert_eq!(val, 42);

        // Second call returns existing value, closure not called
        let val = ext.get_or_insert_with(|| 100u32);
        assert_eq!(val, 42);

        // Works with Arc for shared state
        #[derive(Clone, Default)]
        struct Counter(Arc<AtomicU64>);

        let c1 = ext.get_or_insert_with(Counter::default);
        c1.0.fetch_add(1, Ordering::Relaxed);

        let c2 = ext.get_or_insert_with(Counter::default);
        assert_eq!(c2.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_get_or_default() {
        let ext = Extensions::new();

        #[derive(Clone, Default, PartialEq, Debug)]
        struct MyData(u32);

        // First call creates default value
        let val = ext.get_or_default::<MyData>();
        assert_eq!(val, MyData(0));

        // Insert a different value
        ext.insert(MyData(42));

        // Now returns the inserted value
        let val = ext.get_or_default::<MyData>();
        assert_eq!(val, MyData(42));
    }
}
