use std::fmt::Debug;
use std::future::Future;
use std::time::Duration;

use crate::AsyncRuntime;
use crate::OptionalSend;

use super::AsyncOneshotSendExt;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MonoioRuntime;

pub type MonoioInstant = monoio::time::Instant;

impl crate::Instant for monoio::time::Instant {
    #[inline]
    fn now() -> Self {
        monoio::time::Instant::now()
    }
}

pub struct MonoioOneshotSender<T: OptionalSend>(pub local_sync::oneshot::Sender<T>);

impl AsyncRuntime for MonoioRuntime {
    type JoinError = crate::error::Infallible;
    type JoinHandle<T: OptionalSend + 'static> = monoio::task::JoinHandle<Result<T, Self::JoinError>>;
    type Sleep = monoio::time::Sleep;
    type Instant = MonoioInstant;
    type TimeoutError = monoio::time::error::Elapsed;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = monoio::time::Timeout<T>;
    type ThreadLocalRng = rand::rngs::ThreadRng;
    type OneshotSender<T: OptionalSend> = MonoioOneshotSender<T>;
    type OneshotReceiver<T: OptionalSend> = local_sync::oneshot::Receiver<T>;
    type OneshotReceiverError = local_sync::oneshot::error::RecvError;

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
        monoio::time::sleep_until(deadline)
    }

    #[inline]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F> {
        monoio::time::timeout(duration, future)
    }

    #[inline]
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F> {
        monoio::time::timeout_at(deadline, future)
    }

    #[inline]
    fn is_panic(_join_error: &Self::JoinError) -> bool {
        // A monoio task shouldn't panic or it would bubble the panic in case of a join
        false
    }

    #[inline]
    fn thread_rng() -> Self::ThreadLocalRng {
        rand::thread_rng()
    }

    #[inline]
    fn oneshot<T>() -> (Self::OneshotSender<T>, Self::OneshotReceiver<T>)
    where T: OptionalSend {
        let (tx, rx) = local_sync::oneshot::channel();
        (MonoioOneshotSender(tx), rx)
    }
}

impl<T: OptionalSend> AsyncOneshotSendExt<T> for MonoioOneshotSender<T> {
    #[inline]
    fn send(self, t: T) -> Result<(), T> {
        self.0.send(t)
    }
}

impl<T: OptionalSend> Debug for MonoioOneshotSender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MonoioSendWrapper").finish()
    }
}
