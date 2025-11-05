//! Basic types and traits with optional feature support.
//!
//! This module provides foundational traits that adapt based on feature flags,
//! allowing Openraft to work in both multi-threaded and single-threaded environments.
//!
//! ## Key Traits
//!
//! - [`OptionalSend`] - `Send` when not `singlethreaded`, empty otherwise
//! - [`OptionalSync`] - `Sync` when not `singlethreaded`, empty otherwise
//! - [`OptionalSerde`] - Serde traits when `serde` feature enabled
//! - [`OptionalFeatures`] - Combines all optional traits
//!
//! ## Type Aliases
//!
//! - [`BoxFuture`] - Boxed future, optionally `Send`
//! - [`BoxAsyncOnceMut`] - Boxed async FnOnce with mutable access
//! - [`BoxOnce`] - Boxed FnOnce closure
//! - [`BoxAny`] - Boxed Any type
//!
//! ## Overview
//!
//! These types allow Openraft to be used in:
//! - **Multi-threaded** contexts (default): Types are `Send` + `Sync`
//! - **Single-threaded** contexts (feature `singlethreaded`): No `Send` + `Sync` bounds
//! - **With/without serde** (feature `serde`): Optional serialization support
//!
//! Applications rarely need to use these types directly - they're used internally
//! to make Openraft flexible across different environments.

pub(crate) mod finalized;
pub(crate) mod histogram;

pub use serde_able::OptionalSerde;
pub use threaded::BoxAny;
pub use threaded::BoxAsyncOnceMut;
pub use threaded::BoxFuture;
pub use threaded::BoxMaybeAsyncOnceMut;
pub use threaded::BoxOnce;
pub use threaded::BoxStream;
pub use threaded::OptionalSend;
pub use threaded::OptionalSync;

#[cfg(not(feature = "singlethreaded"))]
mod threaded {
    use std::any::Any;
    use std::future::Future;
    use std::pin::Pin;

    use futures::Stream;

    /// A trait that is empty if the `singlethreaded` feature flag is enabled,
    /// otherwise it extends `Send`.
    pub trait OptionalSend: Send {}
    impl<T: Send + ?Sized> OptionalSend for T {}

    /// A trait that is empty if the `singlethreaded` feature flag is enabled,
    /// otherwise it extends `Sync`.
    pub trait OptionalSync: Sync {}
    impl<T: Sync + ?Sized> OptionalSync for T {}

    /// Type alias for a boxed pinned future that is `Send`.
    pub type BoxFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
    /// Type alias for a boxed pinned stream that is `Send`.
    pub type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + Send + 'a>>;
    /// Type alias for a boxed async function that mutates its argument and is `Send`.
    pub type BoxAsyncOnceMut<'a, A, T = ()> = Box<dyn FnOnce(&mut A) -> BoxFuture<T> + Send + 'a>;
    /// Type alias for a boxed function that optionally returns an async future.
    pub type BoxMaybeAsyncOnceMut<'a, A, T = ()> = Box<dyn FnOnce(&mut A) -> Option<BoxFuture<T>> + Send + 'a>;
    /// Type alias for a boxed function that takes an argument and is `Send`.
    pub type BoxOnce<'a, A, T = ()> = Box<dyn FnOnce(&A) -> T + Send + 'a>;
    /// Type alias for a boxed value that is `Send` and can be any type.
    pub type BoxAny = Box<dyn Any + Send>;
}

#[cfg(feature = "singlethreaded")]
mod threaded {
    use std::any::Any;
    use std::future::Future;
    use std::pin::Pin;

    use futures::Stream;

    /// A trait that is empty if the `singlethreaded` feature flag is enabled,
    /// otherwise it extends `Send`.
    pub trait OptionalSend {}
    impl<T: ?Sized> OptionalSend for T {}

    /// A trait that is empty if the `singlethreaded` feature flag is enabled,
    /// otherwise it extends `Sync`.
    pub trait OptionalSync {}
    impl<T: ?Sized> OptionalSync for T {}

    pub type BoxFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + 'a>>;
    pub type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + 'a>>;
    pub type BoxAsyncOnceMut<'a, A, T = ()> = Box<dyn FnOnce(&mut A) -> BoxFuture<T> + 'a>;
    pub type BoxMaybeAsyncOnceMut<'a, A, T = ()> = Box<dyn FnOnce(&mut A) -> Option<BoxFuture<T>> + 'a>;
    pub type BoxOnce<'a, A, T = ()> = Box<dyn FnOnce(&A) -> T + 'a>;
    pub type BoxAny = Box<dyn Any>;
}

#[cfg(not(feature = "serde"))]
mod serde_able {
    /// A trait that extends `Serialize` and `Deserialize` if the `serde` feature flag
    /// is enabled, otherwise it is an empty trait.
    pub trait OptionalSerde {}
    impl<T> OptionalSerde for T {}
}

#[cfg(feature = "serde")]
mod serde_able {
    /// A trait that extends `Serialize` and `Deserialize` if the `serde` feature flag
    /// is enabled, otherwise it is an empty trait.
    pub trait OptionalSerde: serde::Serialize + for<'a> serde::Deserialize<'a> {}
    impl<T> OptionalSerde for T where T: serde::Serialize + for<'a> serde::Deserialize<'a> {}
}

/// A trait that combines all optional features.
pub trait OptionalFeatures: OptionalSend + OptionalSync + OptionalSerde {}

impl<T> OptionalFeatures for T where T: OptionalSend + OptionalSync + OptionalSerde {}
