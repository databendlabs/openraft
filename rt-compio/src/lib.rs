use std::any::Any;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Error;
use std::fmt::Formatter;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

pub use compio;
pub use futures;
use futures::FutureExt;
pub use openraft;
use openraft::AsyncRuntime;
use openraft::OptionalSend;
pub use rand;
use rand::rngs::ThreadRng;

use crate::mpsc::FlumeMpsc;
use crate::mpsc_unbounded::FlumeMpscUnbounded;
use crate::oneshot::FuturesOneshot;
use crate::watch::See;

mod mpsc;
mod mpsc_unbounded;
mod mutex;
mod oneshot;
mod watch;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompioRuntime;

#[derive(Debug)]
pub struct CompioJoinError(#[allow(dead_code)] Box<dyn Any + Send>);

impl Display for CompioJoinError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        write!(f, "Spawned task panicked")
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

impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Time has elapsed")
    }
}

impl<F: Future> Future for CompioTimeout<F> {
    type Output = Result<F::Output, Elapsed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.delay.poll_unpin(cx) {
            Poll::Ready(()) => {
                // The delay has elapsed, so we return an error.
                Poll::Ready(Err(Elapsed(())))
            }
            Poll::Pending => {
                // The delay has not yet elapsed, so we poll the future.
                match this.future.poll(cx) {
                    Poll::Ready(v) => Poll::Ready(Ok(v)),
                    Poll::Pending => Poll::Pending,
                }
            }
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
    type MpscUnbounded = FlumeMpscUnbounded;
    type Watch = See;
    type Oneshot = FuturesOneshot;
    type Mutex<T: OptionalSend + 'static> = mutex::FlumeMutex<T>;

    fn spawn<T>(fut: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        CompioJoinHandle(Some(compio::runtime::spawn(fut)))
    }

    fn sleep(duration: std::time::Duration) -> Self::Sleep {
        Box::pin(compio::time::sleep(duration))
    }

    fn sleep_until(deadline: Self::Instant) -> Self::Sleep {
        Box::pin(compio::time::sleep_until(deadline))
    }

    fn timeout<R, F: Future<Output = R> + OptionalSend>(
        duration: std::time::Duration,
        future: F,
    ) -> Self::Timeout<R, F> {
        let delay = Box::pin(compio::time::sleep(duration));
        CompioTimeout { future, delay }
    }

    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F> {
        let delay = Box::pin(compio::time::sleep_until(deadline));
        CompioTimeout { future, delay }
    }

    fn is_panic(_: &Self::JoinError) -> bool {
        // Task only returns `JoinError` if the spawned future panics.
        true
    }

    fn thread_rng() -> Self::ThreadLocalRng {
        rand::rng()
    }
}

#[cfg(test)]
mod tests {
    use openraft::testing::runtime::Suite;

    use super::*;

    #[test]
    fn test_compio_rt() {
        let rt = compio::runtime::Runtime::new().unwrap();
        rt.block_on(Suite::<CompioRuntime>::test_all());
    }
}
