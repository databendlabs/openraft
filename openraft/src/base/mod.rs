//! Basic types used in the Raft implementation.

pub(crate) mod finalized;

pub use serde_able::OptionalSerde;
pub use threaded::BoxAny;
pub use threaded::BoxAsyncOnceMut;
pub use threaded::BoxFuture;
pub use threaded::BoxMaybeAsyncOnceMut;
pub use threaded::BoxOnce;
pub use threaded::OptionalSend;
pub use threaded::OptionalSync;

#[cfg(not(feature = "singlethreaded"))]
mod threaded {
    use std::any::Any;
    use std::future::Future;
    use std::pin::Pin;

    /// A trait that is empty if the `singlethreaded` feature flag is enabled,
    /// otherwise it extends `Send`.
    pub trait OptionalSend: Send {}
    impl<T: Send + ?Sized> OptionalSend for T {}

    /// A trait that is empty if the `singlethreaded` feature flag is enabled,
    /// otherwise it extends `Sync`.
    pub trait OptionalSync: Sync {}
    impl<T: Sync + ?Sized> OptionalSync for T {}

    pub type BoxFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
    pub type BoxAsyncOnceMut<'a, A, T = ()> = Box<dyn FnOnce(&mut A) -> BoxFuture<T> + Send + 'a>;
    pub type BoxMaybeAsyncOnceMut<'a, A, T = ()> = Box<dyn FnOnce(&mut A) -> Option<BoxFuture<T>> + Send + 'a>;
    pub type BoxOnce<'a, A, T = ()> = Box<dyn FnOnce(&A) -> T + Send + 'a>;
    pub type BoxAny = Box<dyn Any + Send>;
}

#[cfg(feature = "singlethreaded")]
mod threaded {
    use std::any::Any;
    use std::future::Future;
    use std::pin::Pin;

    /// A trait that is empty if the `singlethreaded` feature flag is enabled,
    /// otherwise it extends `Send`.
    pub trait OptionalSend {}
    impl<T: ?Sized> OptionalSend for T {}

    /// A trait that is empty if the `singlethreaded` feature flag is enabled,
    /// otherwise it extends `Sync`.
    pub trait OptionalSync {}
    impl<T: ?Sized> OptionalSync for T {}

    pub type BoxFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + 'a>>;
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
