use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::sync::watch as tokio_watch;

use crate::AsyncRuntime;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::TokioInstant;
use crate::async_runtime::mpsc_unbounded;
use crate::async_runtime::mpsc_unbounded::MpscUnbounded;
use crate::async_runtime::mutex;
use crate::async_runtime::oneshot;
use crate::async_runtime::watch;
use crate::type_config::OneshotSender;

/// `Tokio` is the default asynchronous executor.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TokioRuntime;

impl AsyncRuntime for TokioRuntime {
    type JoinError = tokio::task::JoinError;
    type JoinHandle<T: OptionalSend + 'static> = tokio::task::JoinHandle<T>;
    type Sleep = tokio::time::Sleep;
    type Instant = TokioInstant;
    type TimeoutError = tokio::time::error::Elapsed;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = tokio::time::Timeout<T>;
    type ThreadLocalRng = rand::rngs::ThreadRng;

    #[inline]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        #[cfg(feature = "singlethreaded")]
        {
            tokio::task::spawn_local(future)
        }
        #[cfg(not(feature = "singlethreaded"))]
        {
            tokio::task::spawn(future)
        }
    }

    #[inline]
    fn sleep(duration: Duration) -> Self::Sleep {
        tokio::time::sleep(duration)
    }

    #[inline]
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep {
        tokio::time::sleep_until(deadline)
    }

    #[inline]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F> {
        tokio::time::timeout(duration, future)
    }

    #[inline]
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F> {
        tokio::time::timeout_at(deadline, future)
    }

    #[inline]
    fn is_panic(join_error: &Self::JoinError) -> bool {
        join_error.is_panic()
    }

    #[inline]
    fn thread_rng() -> Self::ThreadLocalRng {
        rand::rng()
    }

    type Mpsc = mpsc_impl::TokioMpsc;
    type MpscUnbounded = TokioMpscUnbounded;
    type Watch = TokioWatch;
    type Oneshot = TokioOneshot;
    type Mutex<T: OptionalSend + 'static> = TokioMutex<T>;
}

pub struct TokioMpscUnbounded;

impl MpscUnbounded for TokioMpscUnbounded {
    type Sender<T: OptionalSend> = mpsc::UnboundedSender<T>;
    type Receiver<T: OptionalSend> = mpsc::UnboundedReceiver<T>;
    type WeakSender<T: OptionalSend> = mpsc::WeakUnboundedSender<T>;

    /// Creates an unbounded mpsc channel for communicating between asynchronous
    /// tasks without backpressure.
    fn channel<T: OptionalSend>() -> (Self::Sender<T>, Self::Receiver<T>) {
        mpsc::unbounded_channel()
    }
}

impl<T> mpsc_unbounded::MpscUnboundedSender<TokioMpscUnbounded, T> for mpsc::UnboundedSender<T>
where T: OptionalSend
{
    #[inline]
    fn send(&self, msg: T) -> Result<(), mpsc_unbounded::SendError<T>> {
        self.send(msg).map_err(|e| mpsc_unbounded::SendError(e.0))
    }

    #[inline]
    fn downgrade(&self) -> <TokioMpscUnbounded as MpscUnbounded>::WeakSender<T> {
        self.downgrade()
    }
}

impl<T> mpsc_unbounded::MpscUnboundedReceiver<T> for mpsc::UnboundedReceiver<T>
where T: OptionalSend
{
    #[inline]
    async fn recv(&mut self) -> Option<T> {
        self.recv().await
    }

    #[inline]
    fn try_recv(&mut self) -> Result<T, mpsc_unbounded::TryRecvError> {
        self.try_recv().map_err(|e| match e {
            mpsc::error::TryRecvError::Empty => mpsc_unbounded::TryRecvError::Empty,
            mpsc::error::TryRecvError::Disconnected => mpsc_unbounded::TryRecvError::Disconnected,
        })
    }
}

impl<T> mpsc_unbounded::MpscUnboundedWeakSender<TokioMpscUnbounded, T> for mpsc::WeakUnboundedSender<T>
where T: OptionalSend
{
    #[inline]
    fn upgrade(&self) -> Option<<TokioMpscUnbounded as MpscUnbounded>::Sender<T>> {
        self.upgrade()
    }
}

mod mpsc_impl {
    use std::future::Future;

    use futures::TryFutureExt;
    use tokio::sync::mpsc;

    use crate::OptionalSend;
    use crate::async_runtime::Mpsc;
    use crate::async_runtime::MpscReceiver;
    use crate::async_runtime::MpscSender;
    use crate::async_runtime::MpscWeakSender;
    use crate::async_runtime::SendError;
    use crate::async_runtime::TryRecvError;

    pub struct TokioMpsc;

    impl Mpsc for TokioMpsc {
        type Sender<T: OptionalSend> = mpsc::Sender<T>;
        type Receiver<T: OptionalSend> = mpsc::Receiver<T>;
        type WeakSender<T: OptionalSend> = mpsc::WeakSender<T>;

        /// Creates a bounded mpsc channel for communicating between asynchronous
        /// tasks with backpressure.
        fn channel<T: OptionalSend>(buffer: usize) -> (Self::Sender<T>, Self::Receiver<T>) {
            mpsc::channel(buffer)
        }
    }

    impl<T> MpscSender<TokioMpsc, T> for mpsc::Sender<T>
    where T: OptionalSend
    {
        #[inline]
        fn send(&self, msg: T) -> impl Future<Output = Result<(), SendError<T>>> + OptionalSend {
            self.send(msg).map_err(|e| SendError(e.0))
        }

        #[inline]
        fn downgrade(&self) -> <TokioMpsc as Mpsc>::WeakSender<T> {
            self.downgrade()
        }
    }

    impl<T> MpscReceiver<T> for mpsc::Receiver<T>
    where T: OptionalSend
    {
        #[inline]
        fn recv(&mut self) -> impl Future<Output = Option<T>> + OptionalSend {
            self.recv()
        }

        #[inline]
        fn try_recv(&mut self) -> Result<T, TryRecvError> {
            self.try_recv().map_err(|e| match e {
                mpsc::error::TryRecvError::Empty => TryRecvError::Empty,
                mpsc::error::TryRecvError::Disconnected => TryRecvError::Disconnected,
            })
        }
    }

    impl<T> MpscWeakSender<TokioMpsc, T> for mpsc::WeakSender<T>
    where T: OptionalSend
    {
        #[inline]
        fn upgrade(&self) -> Option<<TokioMpsc as Mpsc>::Sender<T>> {
            self.upgrade()
        }
    }
}

pub struct TokioWatch;

impl watch::Watch for TokioWatch {
    type Sender<T: OptionalSend + OptionalSync> = tokio_watch::Sender<T>;
    type Receiver<T: OptionalSend + OptionalSync> = tokio_watch::Receiver<T>;

    type Ref<'a, T: OptionalSend + 'a> = tokio_watch::Ref<'a, T>;

    fn channel<T: OptionalSend + OptionalSync>(init: T) -> (Self::Sender<T>, Self::Receiver<T>) {
        tokio_watch::channel(init)
    }
}

impl<T> watch::WatchSender<TokioWatch, T> for tokio_watch::Sender<T>
where T: OptionalSend + OptionalSync
{
    fn send(&self, value: T) -> Result<(), watch::SendError<T>> {
        self.send(value).map_err(|e| watch::SendError(e.0))
    }

    fn send_if_modified<F>(&self, modify: F) -> bool
    where F: FnOnce(&mut T) -> bool {
        self.send_if_modified(modify)
    }

    fn borrow_watched(&self) -> <TokioWatch as watch::Watch>::Ref<'_, T> {
        self.borrow()
    }
}

impl<T> watch::WatchReceiver<TokioWatch, T> for tokio_watch::Receiver<T>
where T: OptionalSend + OptionalSync
{
    async fn changed(&mut self) -> Result<(), watch::RecvError> {
        self.changed().await.map_err(|_| watch::RecvError(()))
    }

    fn borrow_watched(&self) -> <TokioWatch as watch::Watch>::Ref<'_, T> {
        self.borrow()
    }
}

pub struct TokioOneshot;

impl oneshot::Oneshot for TokioOneshot {
    type Sender<T: OptionalSend> = tokio::sync::oneshot::Sender<T>;
    type Receiver<T: OptionalSend> = tokio::sync::oneshot::Receiver<T>;
    type ReceiverError = tokio::sync::oneshot::error::RecvError;

    #[inline]
    fn channel<T>() -> (Self::Sender<T>, Self::Receiver<T>)
    where T: OptionalSend {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (tx, rx)
    }
}

impl<T> OneshotSender<T> for tokio::sync::oneshot::Sender<T>
where T: OptionalSend
{
    #[inline]
    fn send(self, t: T) -> Result<(), T> {
        self.send(t)
    }
}

type TokioMutex<T> = tokio::sync::Mutex<T>;

impl<T> mutex::Mutex<T> for TokioMutex<T>
where T: OptionalSend + 'static
{
    type Guard<'a> = tokio::sync::MutexGuard<'a, T>;

    fn new(value: T) -> Self {
        TokioMutex::new(value)
    }

    fn lock(&self) -> impl Future<Output = Self::Guard<'_>> + OptionalSend {
        self.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::runtime::Suite;

    #[test]
    #[cfg(not(feature = "singlethreaded"))]
    fn test_tokio_rt_not_singlethreaded() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(8)
            .enable_all()
            .build()
            .expect("Failed building the runtime");

        rt.block_on(Suite::<TokioRuntime>::test_all());
    }

    #[test]
    #[cfg(feature = "singlethreaded")]
    fn test_tokio_rt_singlethreaded() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(8)
            .enable_all()
            .build()
            .expect("Failed building the runtime");
        // `spawn_local` needs to be called called from inside of a `task::LocalSet`
        let local = tokio::task::LocalSet::new();

        local.block_on(&rt, Suite::<TokioRuntime>::test_all());
    }
}
