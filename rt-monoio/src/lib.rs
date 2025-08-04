//! This crate provides a [`MonoioRuntime`] type, which has [`AsyncRuntime`]
//! implemented so that you can use Openraft with [Monoio](monoio).
//!
//! ```ignore
//! pub struct TypeConfig {}
//!
//! impl openraft::RaftTypeConfig for TypeConfig {
//!     // Other type are omitted
//!
//!     type AsyncRuntime = openraft_rt_monoio::MonoioRuntime;
//! }
//! ```
//!
//! # NOTE
//!
//! 1. For the Openraft dependency used with this crate
//!    1. You can disable the `default` feature as you don't need the built-in Tokio runtime.
//!    2. The `singlethreaded` feature needs to be enabled or this crate won't work.
//! 2. With the `singlethreaded` feature enabled, the handle type [`Raft`](openraft::Raft) will be
//!    no longer [`Send`] and [`Sync`].
//! 3. Even though this crate allows you to use Monoio, it still uses some primitives from Tokio
//!    1. `MpscUnbounded`: Monoio (or `local_sync`)'s unbounded MPSC implementation does not have a
//!       weak sender.
//!    2. `Watch`: Monoio (or `local_sync`) does not have a watch channel.
//!    3. `Mutex`: Monoio does not provide a Mutex implementation.

use std::future::Future;
use std::time::Duration;

use openraft::AsyncRuntime;
use openraft::OptionalSend;

/// [`AsyncRuntime`] implementation for Monoio.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MonoioRuntime;

impl AsyncRuntime for MonoioRuntime {
    // Joining an async task on Monoio always succeeds
    type JoinError = openraft::error::Infallible;
    type JoinHandle<T: OptionalSend + 'static> = monoio::task::JoinHandle<Result<T, Self::JoinError>>;
    type Sleep = monoio::time::Sleep;
    type Instant = instant_mod::MonoioInstant;
    type TimeoutError = monoio::time::error::Elapsed;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = monoio::time::Timeout<T>;
    type ThreadLocalRng = rand::rngs::ThreadRng;

    #[inline]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        monoio::spawn(async move { Ok(future.await) })
    }

    #[inline]
    fn sleep(duration: Duration) -> Self::Sleep {
        monoio::time::sleep(duration)
    }

    #[inline]
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep {
        monoio::time::sleep_until(deadline.0)
    }

    #[inline]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F> {
        monoio::time::timeout(duration, future)
    }

    #[inline]
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F> {
        monoio::time::timeout_at(deadline.0, future)
    }

    #[inline]
    fn is_panic(join_error: &Self::JoinError) -> bool {
        match *join_error {}
    }

    #[inline]
    fn thread_rng() -> Self::ThreadLocalRng {
        rand::rng()
    }

    type Mpsc = mpsc_mod::MonoioMpsc;
    type MpscUnbounded = mpsc_unbounded_mod::TokioMpscUnbounded;
    type Watch = watch_mod::TokioWatch;
    type Oneshot = oneshot_mod::MonoioOneshot;
    type Mutex<T: OptionalSend + 'static> = mutex_mod::TokioMutex<T>;
}

// Put the wrapper types in a private module to make them `pub` but not
// exposed to the user.
mod instant_mod {
    //! Instant channel wrapper type and its trait impl.

    use std::ops::Add;
    use std::ops::AddAssign;
    use std::ops::Sub;
    use std::ops::SubAssign;
    use std::time::Duration;

    use openraft::instant;

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
    pub struct MonoioInstant(pub(crate) monoio::time::Instant);

    impl Add<Duration> for MonoioInstant {
        type Output = Self;

        #[inline]
        fn add(self, rhs: Duration) -> Self::Output {
            Self(self.0.add(rhs))
        }
    }

    impl AddAssign<Duration> for MonoioInstant {
        #[inline]
        fn add_assign(&mut self, rhs: Duration) {
            self.0.add_assign(rhs)
        }
    }

    impl Sub<Duration> for MonoioInstant {
        type Output = Self;

        #[inline]
        fn sub(self, rhs: Duration) -> Self::Output {
            Self(self.0.sub(rhs))
        }
    }

    impl Sub<Self> for MonoioInstant {
        type Output = Duration;

        #[inline]
        fn sub(self, rhs: Self) -> Self::Output {
            self.0.sub(rhs.0)
        }
    }

    impl SubAssign<Duration> for MonoioInstant {
        #[inline]
        fn sub_assign(&mut self, rhs: Duration) {
            self.0.sub_assign(rhs)
        }
    }

    impl instant::Instant for MonoioInstant {
        #[inline]
        fn now() -> Self {
            let inner = monoio::time::Instant::now();
            Self(inner)
        }

        #[inline]
        fn elapsed(&self) -> Duration {
            self.0.elapsed()
        }
    }
}

// Put the wrapper types in a private module to make them `pub` but not
// exposed to the user.
mod oneshot_mod {
    //! Oneshot channel wrapper types and their trait impl.

    use local_sync::oneshot as monoio_oneshot;
    use openraft::type_config::async_runtime::oneshot;
    use openraft::OptionalSend;

    pub struct MonoioOneshot;

    pub struct MonoioOneshotSender<T>(monoio_oneshot::Sender<T>);

    impl oneshot::Oneshot for MonoioOneshot {
        type Sender<T: OptionalSend> = MonoioOneshotSender<T>;
        type Receiver<T: OptionalSend> = monoio_oneshot::Receiver<T>;
        type ReceiverError = monoio_oneshot::error::RecvError;

        #[inline]
        fn channel<T>() -> (Self::Sender<T>, Self::Receiver<T>)
        where T: OptionalSend {
            let (tx, rx) = monoio_oneshot::channel();
            let tx_wrapper = MonoioOneshotSender(tx);

            (tx_wrapper, rx)
        }
    }

    impl<T> oneshot::OneshotSender<T> for MonoioOneshotSender<T>
    where T: OptionalSend
    {
        #[inline]
        fn send(self, t: T) -> Result<(), T> {
            self.0.send(t)
        }
    }
}

// Put the wrapper types in a private module to make them `pub` but not
// exposed to the user.
/// MPSC channel is implemented with tokio MPSC channels.
///
/// Tokio MPSC channel are runtime independent.
mod mpsc_mod {
    //! MPSC channel wrapper types and their trait impl.

    use std::future::Future;

    use futures::TryFutureExt;
    use openraft::async_runtime::Mpsc;
    use openraft::async_runtime::MpscReceiver;
    use openraft::async_runtime::MpscSender;
    use openraft::async_runtime::MpscWeakSender;
    use openraft::async_runtime::SendError;
    use openraft::async_runtime::TryRecvError;
    use openraft::OptionalSend;
    use tokio::sync::mpsc as tokio_mpsc;

    pub struct MonoioMpsc;

    pub struct MonoioMpscSender<T>(tokio_mpsc::Sender<T>);

    impl<T> Clone for MonoioMpscSender<T> {
        #[inline]
        fn clone(&self) -> Self {
            Self(self.0.clone())
        }
    }

    pub struct MonoioMpscReceiver<T>(tokio_mpsc::Receiver<T>);

    pub struct MonoioMpscWeakSender<T>(tokio_mpsc::WeakSender<T>);

    impl<T> Clone for MonoioMpscWeakSender<T> {
        #[inline]
        fn clone(&self) -> Self {
            Self(self.0.clone())
        }
    }

    impl Mpsc for MonoioMpsc {
        type Sender<T: OptionalSend> = MonoioMpscSender<T>;
        type Receiver<T: OptionalSend> = MonoioMpscReceiver<T>;
        type WeakSender<T: OptionalSend> = MonoioMpscWeakSender<T>;

        #[inline]
        fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
            let (tx, rx) = tokio_mpsc::channel(buffer);
            let tx_wrapper = MonoioMpscSender(tx);
            let rx_wrapper = MonoioMpscReceiver(rx);

            (tx_wrapper, rx_wrapper)
        }
    }

    impl<T> MpscSender<MonoioMpsc, T> for MonoioMpscSender<T>
    where T: OptionalSend
    {
        #[inline]
        fn send(&self, msg: T) -> impl Future<Output = Result<(), SendError<T>>> {
            self.0.send(msg).map_err(|e| SendError(e.0))
        }

        #[inline]
        fn downgrade(&self) -> <MonoioMpsc as Mpsc>::WeakSender<T> {
            let inner = self.0.downgrade();
            MonoioMpscWeakSender(inner)
        }
    }

    impl<T> MpscReceiver<T> for MonoioMpscReceiver<T> {
        #[inline]
        fn recv(&mut self) -> impl Future<Output = Option<T>> {
            self.0.recv()
        }

        #[inline]
        fn try_recv(&mut self) -> Result<T, TryRecvError> {
            self.0.try_recv().map_err(|e| match e {
                tokio_mpsc::error::TryRecvError::Empty => TryRecvError::Empty,
                tokio_mpsc::error::TryRecvError::Disconnected => TryRecvError::Disconnected,
            })
        }
    }

    impl<T> MpscWeakSender<MonoioMpsc, T> for MonoioMpscWeakSender<T>
    where T: OptionalSend
    {
        #[inline]
        fn upgrade(&self) -> Option<<MonoioMpsc as Mpsc>::Sender<T>> {
            self.0.upgrade().map(MonoioMpscSender)
        }
    }
}

// Put the wrapper types in a private module to make them `pub` but not
// exposed to the user.
mod mpsc_unbounded_mod {
    //! Unbounded MPSC channel wrapper types and their trait impl.

    use openraft::type_config::async_runtime::mpsc_unbounded;
    use openraft::OptionalSend;
    use tokio::sync::mpsc as tokio_mpsc;

    pub struct TokioMpscUnbounded;

    pub struct TokioMpscUnboundedSender<T>(tokio_mpsc::UnboundedSender<T>);

    impl<T> Clone for TokioMpscUnboundedSender<T> {
        #[inline]
        fn clone(&self) -> Self {
            Self(self.0.clone())
        }
    }

    pub struct TokioMpscUnboundedReceiver<T>(tokio_mpsc::UnboundedReceiver<T>);

    pub struct TokioMpscUnboundedWeakSender<T>(tokio_mpsc::WeakUnboundedSender<T>);

    impl<T> Clone for TokioMpscUnboundedWeakSender<T> {
        #[inline]
        fn clone(&self) -> Self {
            Self(self.0.clone())
        }
    }

    impl mpsc_unbounded::MpscUnbounded for TokioMpscUnbounded {
        type Sender<T: OptionalSend> = TokioMpscUnboundedSender<T>;
        type Receiver<T: OptionalSend> = TokioMpscUnboundedReceiver<T>;
        type WeakSender<T: OptionalSend> = TokioMpscUnboundedWeakSender<T>;

        #[inline]
        fn channel<T: OptionalSend>() -> (Self::Sender<T>, Self::Receiver<T>) {
            let (tx, rx) = tokio_mpsc::unbounded_channel();
            let tx_wrapper = TokioMpscUnboundedSender(tx);
            let rx_wrapper = TokioMpscUnboundedReceiver(rx);

            (tx_wrapper, rx_wrapper)
        }
    }

    impl<T> mpsc_unbounded::MpscUnboundedSender<TokioMpscUnbounded, T> for TokioMpscUnboundedSender<T>
    where T: OptionalSend
    {
        #[inline]
        fn send(&self, msg: T) -> Result<(), mpsc_unbounded::SendError<T>> {
            self.0.send(msg).map_err(|e| mpsc_unbounded::SendError(e.0))
        }

        #[inline]
        fn downgrade(&self) -> <TokioMpscUnbounded as mpsc_unbounded::MpscUnbounded>::WeakSender<T> {
            let inner = self.0.downgrade();
            TokioMpscUnboundedWeakSender(inner)
        }
    }

    impl<T> mpsc_unbounded::MpscUnboundedReceiver<T> for TokioMpscUnboundedReceiver<T> {
        #[inline]
        async fn recv(&mut self) -> Option<T> {
            self.0.recv().await
        }

        #[inline]
        fn try_recv(&mut self) -> Result<T, mpsc_unbounded::TryRecvError> {
            self.0.try_recv().map_err(|e| match e {
                tokio_mpsc::error::TryRecvError::Empty => mpsc_unbounded::TryRecvError::Empty,
                tokio_mpsc::error::TryRecvError::Disconnected => mpsc_unbounded::TryRecvError::Disconnected,
            })
        }
    }

    impl<T> mpsc_unbounded::MpscUnboundedWeakSender<TokioMpscUnbounded, T> for TokioMpscUnboundedWeakSender<T>
    where T: OptionalSend
    {
        #[inline]
        fn upgrade(&self) -> Option<<TokioMpscUnbounded as mpsc_unbounded::MpscUnbounded>::Sender<T>> {
            self.0.upgrade().map(TokioMpscUnboundedSender)
        }
    }
}

// Put the wrapper types in a private module to make them `pub` but not
// exposed to the user.
mod watch_mod {
    //! Watch channel wrapper types and their trait impl.

    use std::ops::Deref;

    use openraft::async_runtime::watch::RecvError;
    use openraft::async_runtime::watch::SendError;
    use openraft::type_config::async_runtime::watch;
    use openraft::OptionalSend;
    use openraft::OptionalSync;
    use tokio::sync::watch as tokio_watch;

    pub struct TokioWatch;
    pub struct TokioWatchSender<T>(tokio_watch::Sender<T>);
    pub struct TokioWatchReceiver<T>(tokio_watch::Receiver<T>);
    pub struct TokioWatchRef<'a, T>(tokio_watch::Ref<'a, T>);

    impl watch::Watch for TokioWatch {
        type Sender<T: OptionalSend + OptionalSync> = TokioWatchSender<T>;
        type Receiver<T: OptionalSend + OptionalSync> = TokioWatchReceiver<T>;
        type Ref<'a, T: OptionalSend + 'a> = TokioWatchRef<'a, T>;

        #[inline]
        fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>) {
            let (tx, rx) = tokio_watch::channel(init);
            let tx_wrapper = TokioWatchSender(tx);
            let rx_wrapper = TokioWatchReceiver(rx);

            (tx_wrapper, rx_wrapper)
        }
    }

    impl<T> watch::WatchSender<TokioWatch, T> for TokioWatchSender<T>
    where T: OptionalSend + OptionalSync
    {
        #[inline]
        fn send(&self, value: T) -> Result<(), SendError<T>> {
            self.0.send(value).map_err(|e| watch::SendError(e.0))
        }

        #[inline]
        fn send_if_modified<F>(&self, modify: F) -> bool
        where F: FnOnce(&mut T) -> bool {
            self.0.send_if_modified(modify)
        }

        #[inline]
        fn borrow_watched(&self) -> <TokioWatch as watch::Watch>::Ref<'_, T> {
            let inner = self.0.borrow();
            TokioWatchRef(inner)
        }
    }

    impl<T> Clone for TokioWatchReceiver<T> {
        #[inline]
        fn clone(&self) -> Self {
            Self(self.0.clone())
        }
    }

    impl<T> watch::WatchReceiver<TokioWatch, T> for TokioWatchReceiver<T>
    where T: OptionalSend + OptionalSync
    {
        #[inline]
        async fn changed(&mut self) -> Result<(), RecvError> {
            self.0.changed().await.map_err(|_| watch::RecvError(()))
        }

        #[inline]
        fn borrow_watched(&self) -> <TokioWatch as watch::Watch>::Ref<'_, T> {
            TokioWatchRef(self.0.borrow())
        }
    }

    impl<'a, T> Deref for TokioWatchRef<'a, T> {
        type Target = T;

        #[inline]
        fn deref(&self) -> &Self::Target {
            self.0.deref()
        }
    }
}

// Put the wrapper types in a private module to make them `pub` but not
// exposed to the user.
mod mutex_mod {
    //! Mutex wrapper type and its trait impl.

    use std::future::Future;

    use openraft::type_config::async_runtime::mutex;
    use openraft::OptionalSend;

    pub struct TokioMutex<T>(tokio::sync::Mutex<T>);

    impl<T> mutex::Mutex<T> for TokioMutex<T>
    where T: OptionalSend + 'static
    {
        type Guard<'a> = tokio::sync::MutexGuard<'a, T>;

        #[inline]
        fn new(value: T) -> Self {
            TokioMutex(tokio::sync::Mutex::new(value))
        }

        #[inline]
        fn lock(&self) -> impl Future<Output = Self::Guard<'_>> + OptionalSend {
            self.0.lock()
        }
    }
}

#[cfg(test)]
mod tests {
    use openraft::testing::runtime::Suite;

    use super::*;

    #[test]
    fn test_monoio_rt() {
        let mut rt = monoio::RuntimeBuilder::<monoio::FusionDriver>::new().enable_all().build().unwrap();
        rt.block_on(Suite::<MonoioRuntime>::test_all());
    }
}
