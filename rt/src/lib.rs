//! Async runtime abstraction traits for Openraft.
//!
//! This crate provides the core traits for async runtime abstraction,
//! allowing Openraft to work with different async runtimes (tokio, compio, monoio, etc.).
//!
//! ## Key Traits
//!
//! - [`AsyncRuntime`] - Main runtime abstraction
//! - [`Instant`] - Time measurement
//! - [`Mpsc`], [`MpscSender`], [`MpscReceiver`] - Bounded MPSC channels
//! - [`Mutex`] - Async mutex
//! - [`Oneshot`], [`OneshotSender`] - One-shot channels
//! - [`Watch`], [`WatchSender`], [`WatchReceiver`] - Watch channels
//!
//! ## Features
//!
//! - `single-threaded` - Disables `Send` + `Sync` bounds on [`OptionalSend`] and [`OptionalSync`]

mod async_runtime;
pub mod instant;
pub mod mpsc;
pub mod mutex;
pub mod oneshot;
pub mod testing;
pub mod watch;

pub use async_runtime::AsyncRuntime;
pub use instant::Instant;
pub use mpsc::Mpsc;
pub use mpsc::MpscReceiver;
pub use mpsc::MpscSender;
pub use mpsc::MpscWeakSender;
pub use mpsc::SendError;
pub use mpsc::TryRecvError;
pub use mutex::Mutex;
pub use oneshot::Oneshot;
pub use oneshot::OneshotSender;
pub use threaded::BoxFuture;
pub use threaded::BoxStream;
pub use threaded::OptionalSend;
pub use threaded::OptionalSync;
pub use watch::RecvError;
pub use watch::Watch;
pub use watch::WatchReceiver;
pub use watch::WatchSender;

#[cfg(not(feature = "single-threaded"))]
mod threaded {
    use std::future::Future;
    use std::pin::Pin;

    use futures::Stream;

    /// A trait that is empty if the `single-threaded` feature flag is enabled,
    /// otherwise it extends `Send`.
    pub trait OptionalSend: Send {}
    impl<T: Send + ?Sized> OptionalSend for T {}

    /// A trait that is empty if the `single-threaded` feature flag is enabled,
    /// otherwise it extends `Sync`.
    pub trait OptionalSync: Sync {}
    impl<T: Sync + ?Sized> OptionalSync for T {}

    /// Type alias for a boxed pinned future that is `Send`.
    pub type BoxFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
    /// Type alias for a boxed pinned stream that is `Send`.
    pub type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + Send + 'a>>;
}

#[cfg(feature = "single-threaded")]
mod threaded {
    use std::future::Future;
    use std::pin::Pin;

    use futures::Stream;

    /// A trait that is empty if the `single-threaded` feature flag is enabled,
    /// otherwise it extends `Send`.
    pub trait OptionalSend {}
    impl<T: ?Sized> OptionalSend for T {}

    /// A trait that is empty if the `single-threaded` feature flag is enabled,
    /// otherwise it extends `Sync`.
    pub trait OptionalSync {}
    impl<T: ?Sized> OptionalSync for T {}

    /// Type alias for a boxed pinned future.
    pub type BoxFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + 'a>>;
    /// Type alias for a boxed pinned stream.
    pub type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + 'a>>;
}
