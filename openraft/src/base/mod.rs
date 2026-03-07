//! Basic types and traits with optional feature support.
//!
//! This module provides foundational traits that adapt based on feature flags,
//! allowing Openraft to work in both multi-threaded and single-threaded environments.
//!
//! ## Key Traits
//!
//! - [`OptionalSend`] - `Send` when not `single-threaded`, empty otherwise
//! - [`OptionalSync`] - `Sync` when not `single-threaded`, empty otherwise
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
//! - **Single-threaded** contexts (feature `single-threaded`): No `Send` + `Sync` bounds
//! - **With/without serde** (feature `serde`): Optional serialization support
//!
//! Applications rarely need to use these types directly - they're used internally
//! to make Openraft flexible across different environments.

pub(crate) mod batch;
pub(crate) mod finalized;
pub(crate) mod histogram;
pub(crate) mod shared_id_generator;

pub use batch::RaftBatch;
pub use openraft_rt::BoxAny;
pub use openraft_rt::BoxAsyncOnceMut;
pub use openraft_rt::BoxFuture;
pub use openraft_rt::BoxIterator;
pub use openraft_rt::BoxMaybeAsyncOnceMut;
pub use openraft_rt::BoxOnce;
pub use openraft_rt::BoxStream;
pub use openraft_rt::OptionalSend;
pub use openraft_rt::OptionalSync;
pub use serde_able::OptionalSerde;

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
