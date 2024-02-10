//! Basic types used in the Raft implementation.

pub use serde_able::OptionalSerde;
pub use threaded::BoxAsyncOnceMut;
pub use threaded::BoxFuture;
pub use threaded::BoxOnce;
pub use threaded::OptionalSend;
pub use threaded::OptionalSync;

#[cfg(not(feature = "singlethreaded"))]
mod threaded {
    use std::future::Future;
    use std::pin::Pin;

    pub trait OptionalSend: Send {}
    impl<T: Send + ?Sized> OptionalSend for T {}

    pub trait OptionalSync: Sync {}
    impl<T: Sync + ?Sized> OptionalSync for T {}

    pub type BoxFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
    pub type BoxAsyncOnceMut<'a, A, T = ()> = Box<dyn FnOnce(&mut A) -> BoxFuture<T> + Send + 'a>;
    pub type BoxOnce<'a, A, T = ()> = Box<dyn FnOnce(&A) -> T + Send + 'a>;
}

#[cfg(feature = "singlethreaded")]
mod threaded {
    use std::future::Future;
    use std::pin::Pin;

    pub trait OptionalSend {}
    impl<T: ?Sized> OptionalSend for T {}

    pub trait OptionalSync {}
    impl<T: ?Sized> OptionalSync for T {}

    pub type BoxFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + 'a>>;
    pub type BoxAsyncOnceMut<'a, A, T = ()> = Box<dyn FnOnce(&mut A) -> BoxFuture<T> + 'a>;
    pub type BoxOnce<'a, A, T = ()> = Box<dyn FnOnce(&A) -> T + 'a>;
}

#[cfg(not(feature = "serde"))]
mod serde_able {
    #[doc(hidden)]
    pub trait OptionalSerde {}
    impl<T> OptionalSerde for T {}
}

#[cfg(feature = "serde")]
mod serde_able {
    #[doc(hidden)]
    pub trait OptionalSerde: serde::Serialize + for<'a> serde::Deserialize<'a> {}
    impl<T> OptionalSerde for T where T: serde::Serialize + for<'a> serde::Deserialize<'a> {}
}
