use std::future::Future;
use std::time::Duration;

use openraft_rt::AsyncRuntime;
use openraft_rt::OptionalSend;

mod instant;
mod mpsc;
mod mutex;
mod oneshot;
mod watch;

pub use instant::TokioInstant;
pub use mpsc::TokioMpsc;
pub use mpsc::TokioMpscReceiver;
pub use mpsc::TokioMpscSender;
pub use mpsc::TokioMpscWeakSender;
pub use mutex::TokioMutex;
pub use oneshot::TokioOneshot;
pub use oneshot::TokioOneshotSender;
pub use watch::TokioWatch;
pub use watch::TokioWatchReceiver;
pub use watch::TokioWatchSender;

/// `Tokio` is the default asynchronous executor.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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
        #[cfg(feature = "single-threaded")]
        {
            tokio::task::spawn_local(future)
        }
        #[cfg(not(feature = "single-threaded"))]
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
        tokio::time::sleep_until(deadline.0)
    }

    #[inline]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F> {
        tokio::time::timeout(duration, future)
    }

    #[inline]
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F> {
        tokio::time::timeout_at(deadline.0, future)
    }

    #[inline]
    fn is_panic(join_error: &Self::JoinError) -> bool {
        join_error.is_panic()
    }

    #[inline]
    fn thread_rng() -> Self::ThreadLocalRng {
        rand::rng()
    }

    type Mpsc = TokioMpsc;
    type Watch = TokioWatch;
    type Oneshot = TokioOneshot;
    type Mutex<T: OptionalSend + 'static> = TokioMutex<T>;
}

#[cfg(test)]
mod tests {
    use openraft_rt::testing::Suite;

    use super::*;

    #[test]
    #[cfg(not(feature = "single-threaded"))]
    fn test_tokio_rt_not_single_threaded() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(8)
            .enable_all()
            .build()
            .expect("Failed building the runtime");

        rt.block_on(Suite::<TokioRuntime>::test_all());
    }

    #[test]
    #[cfg(feature = "single-threaded")]
    fn test_tokio_rt_single_threaded() {
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
