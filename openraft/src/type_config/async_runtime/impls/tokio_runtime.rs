use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::async_runtime::mpsc_unbounded;
use crate::async_runtime::mpsc_unbounded::MpscUnbounded;
use crate::type_config::OneshotSender;
use crate::AsyncRuntime;
use crate::OptionalSend;
use crate::TokioInstant;

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
    type OneshotSender<T: OptionalSend> = tokio::sync::oneshot::Sender<T>;
    type OneshotReceiver<T: OptionalSend> = tokio::sync::oneshot::Receiver<T>;
    type OneshotReceiverError = tokio::sync::oneshot::error::RecvError;

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
        rand::thread_rng()
    }

    #[inline]
    fn oneshot<T>() -> (Self::OneshotSender<T>, Self::OneshotReceiver<T>)
    where T: OptionalSend {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (tx, rx)
    }

    type MpscUnbounded = TokioMpscUnbounded;
}

impl<T> OneshotSender<T> for tokio::sync::oneshot::Sender<T> {
    #[inline]
    fn send(self, t: T) -> Result<(), T> {
        self.send(t)
    }
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
