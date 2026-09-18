//! Deadlines around a future.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use pin_project_lite::pin_project;

use crate::sleep::SimSleep;

/// The inner future did not complete before the deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed(());

impl fmt::Display for Elapsed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "deadline has elapsed")
    }
}

impl std::error::Error for Elapsed {}

pin_project! {
    /// Resolves to the inner future's output, or [`Elapsed`] once the deadline passes first.
    ///
    /// The inner future is polled before the timer, so a future that is ready at the deadline wins.
    #[derive(Debug)]
    pub struct SimTimeout<T> {
        #[pin]
        future: T,
        sleep: SimSleep,
    }
}

impl<T> SimTimeout<T> {
    pub(crate) fn new(future: T, sleep: SimSleep) -> Self {
        SimTimeout { future, sleep }
    }
}

impl<T> Future for SimTimeout<T>
where T: Future
{
    type Output = Result<T::Output, Elapsed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        if let Poll::Ready(output) = this.future.poll(cx) {
            return Poll::Ready(Ok(output));
        }
        match Pin::new(this.sleep).poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(Elapsed(()))),
            Poll::Pending => Poll::Pending,
        }
    }
}
