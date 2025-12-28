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
use std::sync::Mutex;

use crate::base::BoxAny;
use crate::base::OptionalSend;

/// A type-map for storing user-defined extension data.
///
/// Values are stored by their [`TypeId`], so each type can have at most one value.
/// Access is thread-safe via internal [`Mutex`].
///
/// Values must implement [`Clone`] and [`Default`]. When retrieved via [`get()`](Self::get),
/// a clone is returned. If no value exists, `T::default()` is inserted and returned.
pub struct Extensions {
    map: Mutex<HashMap<TypeId, BoxAny>>,
}

impl Extensions {
    /// Get a clone of the value of type `T`.
    ///
    /// If no value exists, `T::default()` is inserted and a clone is returned.
    pub fn get<T>(&self) -> T
    where T: OptionalSend + Clone + Default + 'static {
        let mut map = self.map.lock().unwrap();
        let type_id = TypeId::of::<T>();

        if let Some(val) = map.get(&type_id).and_then(|b| b.downcast_ref::<T>()) {
            return val.clone();
        }

        let val = T::default();
        map.insert(type_id, Box::new(val.clone()));
        val
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
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;

    use super::*;

    #[test]
    fn test_get_inserts_default() {
        let ext = Extensions::default();

        #[derive(Clone, Default, PartialEq, Debug)]
        struct MyData(u32);

        // First call creates default value
        let val = ext.get::<MyData>();
        assert_eq!(val, MyData(0));

        // Value is now stored
        assert!(ext.contains::<MyData>());

        // Second call returns same value
        let val2 = ext.get::<MyData>();
        assert_eq!(val2, MyData(0));
    }

    #[test]
    fn test_shared_state_with_arc() {
        let ext = Extensions::default();

        #[derive(Clone, Default)]
        struct Counter(Arc<AtomicU64>);

        // Get clone and mutate
        let counter1 = ext.get::<Counter>();
        counter1.0.fetch_add(1, Ordering::Relaxed);
        counter1.0.fetch_add(1, Ordering::Relaxed);

        // Get another clone - shares the same Arc
        let counter2 = ext.get::<Counter>();
        assert_eq!(counter2.0.load(Ordering::Relaxed), 2);

        // More increments
        counter2.0.fetch_add(1, Ordering::Relaxed);
        assert_eq!(counter1.0.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn test_remove() {
        let ext = Extensions::default();

        #[derive(Clone, Default, PartialEq, Debug)]
        struct MyData(u32);

        // Insert via get
        let _ = ext.get::<MyData>();
        assert!(ext.contains::<MyData>());

        // Remove
        assert_eq!(ext.remove::<MyData>(), Some(MyData(0)));
        assert!(!ext.contains::<MyData>());
        assert!(ext.remove::<MyData>().is_none());
    }
}
