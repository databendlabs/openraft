use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Error;
use std::fmt::Formatter;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

pub use compio;
use compio_executor::JoinError;
pub use futures;
use futures::FutureExt;
use openraft_rt::AsyncRuntime;
use openraft_rt::OptionalSend;
pub use rand;
use rand::rngs::ThreadRng;

use crate::mpsc::FlumeMpsc;
use crate::oneshot::FuturesOneshot;
use crate::watch::See;

mod mpsc;
mod mutex;
mod oneshot;
mod watch;

/// Compio async runtime.
pub struct CompioRuntime {
    rt: compio::runtime::Runtime,
}

impl Debug for CompioRuntime {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.debug_struct("CompioRuntime").finish()
    }
}

/// The error of awaiting a [`CompioJoinHandle`]: the task panicked, or it was cancelled, for
/// example because its runtime was dropped before the task finished.
#[derive(Debug)]
pub struct CompioJoinError(JoinError);

impl Display for CompioJoinError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        Display::fmt(&self.0, f)
    }
}

pub struct CompioJoinHandle<T>(Option<compio::runtime::JoinHandle<T>>);

impl<T> Drop for CompioJoinHandle<T> {
    fn drop(&mut self) {
        let Some(j) = self.0.take() else {
            return;
        };
        j.detach();
    }
}

impl<T> Future for CompioJoinHandle<T> {
    type Output = Result<T, CompioJoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let task = this.0.as_mut().expect("Task has been cancelled");
        match task.poll_unpin(cx) {
            Poll::Ready(Ok(v)) => Poll::Ready(Ok(v)),
            Poll::Ready(Err(e)) => Poll::Ready(Err(CompioJoinError(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub type BoxedFuture<T> = Pin<Box<dyn Future<Output = T>>>;

pin_project_lite::pin_project! {
    pub struct CompioTimeout<F> {
        #[pin]
        future: F,
        delay: BoxedFuture<()>
    }
}

/// Time has elapsed
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Elapsed(());

impl Display for Elapsed {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Time has elapsed")
    }
}

impl<F: Future> Future for CompioTimeout<F> {
    type Output = Result<F::Output, Elapsed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // Poll the future first, as tokio does: a future that is ready when the deadline passes
        // completes with its output.
        if let Poll::Ready(v) = this.future.poll(cx) {
            return Poll::Ready(Ok(v));
        }

        match this.delay.poll_unpin(cx) {
            Poll::Ready(()) => Poll::Ready(Err(Elapsed(()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncRuntime for CompioRuntime {
    type JoinError = CompioJoinError;
    type JoinHandle<T: OptionalSend + 'static> = CompioJoinHandle<T>;
    type Sleep = BoxedFuture<()>;
    type Instant = std::time::Instant;
    type TimeoutError = Elapsed;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = CompioTimeout<T>;
    type ThreadLocalRng = ThreadRng;
    type Mpsc = FlumeMpsc;
    type Watch = See;
    type Oneshot = FuturesOneshot;
    type Mutex<T: OptionalSend + 'static> = mutex::FuturesMutex<T>;

    fn spawn<T>(fut: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        CompioJoinHandle(Some(compio::runtime::spawn(fut)))
    }

    fn sleep(duration: std::time::Duration) -> Self::Sleep {
        // `compio::time::sleep` reads the clock when it is first polled. Take the deadline now, as
        // tokio and monoio do. A deadline too far to represent never fires.
        match std::time::Instant::now().checked_add(duration) {
            Some(deadline) => Self::sleep_until(deadline),
            None => Box::pin(std::future::pending::<()>()),
        }
    }

    fn sleep_until(deadline: Self::Instant) -> Self::Sleep {
        Box::pin(compio::time::sleep_until(deadline))
    }

    fn timeout<R, F: Future<Output = R> + OptionalSend>(
        duration: std::time::Duration,
        future: F,
    ) -> Self::Timeout<R, F> {
        let delay = Self::sleep(duration);
        CompioTimeout { future, delay }
    }

    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F> {
        let delay = Box::pin(compio::time::sleep_until(deadline));
        CompioTimeout { future, delay }
    }

    fn is_panic(join_error: &Self::JoinError) -> bool {
        matches!(join_error.0, JoinError::Panicked(_))
    }

    fn thread_rng() -> Self::ThreadLocalRng {
        rand::rng()
    }

    fn new(_threads: usize) -> Self {
        // Compio is single-threaded, ignores threads parameter
        let rt = compio::runtime::Runtime::new().expect("Failed to create Compio runtime");
        CompioRuntime { rt }
    }

    fn block_on<F, T>(&mut self, future: F) -> T
    where F: Future<Output = T> {
        self.rt.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use openraft_rt::testing::Suite;

    use super::*;

    #[test]
    fn test_compio_rt() {
        Suite::<CompioRuntime>::test_all();
    }

    #[test]
    fn test_join_error_of_panicked_task() {
        let mut rt = CompioRuntime::new(1);
        let res = rt.block_on(async { CompioRuntime::spawn(async { panic!("the spawned task panics") }).await });
        let err = res.unwrap_err();

        assert!(CompioRuntime::is_panic(&err));
        assert_eq!("Task has panicked", err.to_string());
    }

    #[test]
    fn test_join_error_of_cancelled_task() {
        // Dropping a runtime cancels the tasks it has not finished.
        let owner = CompioRuntime::new(1);
        let handle = owner.rt.enter(|| CompioRuntime::spawn(std::future::pending::<()>()));
        drop(owner);

        let mut rt = CompioRuntime::new(1);
        let err = rt.block_on(handle).unwrap_err();

        assert!(!CompioRuntime::is_panic(&err));
        assert_eq!("Task was cancelled", err.to_string());
    }

    #[test]
    fn test_relative_timers_start_at_creation() {
        let mut rt = CompioRuntime::new(1);
        rt.block_on(async {
            let duration = Duration::from_millis(10);
            let sleep = CompioRuntime::sleep(duration);
            let timeout = CompioRuntime::timeout(duration, std::future::pending::<()>());

            // Let the duration pass before either timer is first polled.
            std::thread::sleep(duration * 2);

            assert_eq!(Some(()), sleep.now_or_never());
            assert_eq!(Some(Err(Elapsed(()))), timeout.now_or_never());

            // A deadline too far to represent never fires.
            assert_eq!(None, CompioRuntime::sleep(Duration::MAX).now_or_never());
        });
    }

    #[test]
    fn test_timeout_polls_future_first() {
        let mut rt = CompioRuntime::new(1);
        rt.block_on(async {
            // The deadline has passed by the first poll, but the future is ready, so it wins.
            let res = CompioRuntime::timeout(Duration::ZERO, std::future::ready(1)).now_or_never();
            assert_eq!(Some(Ok(1)), res);

            let res = CompioRuntime::timeout_at(Instant::now(), std::future::ready(2)).now_or_never();
            assert_eq!(Some(Ok(2)), res);
        });
    }
}
