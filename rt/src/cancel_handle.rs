use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;

use futures_util::task::AtomicWaker;
use openraft_macros::since;

use crate::AsyncRuntime;

/// Signals cancellation of a future created by [`AsyncRuntime::cancellable`].
///
/// Dropping the handle also signals cancellation. The caller drives the wrapped future.
#[since(version = "0.10.1")]
pub struct CancelHandle<R>
where R: AsyncRuntime
{
    /// Shared with the future returned by [`Self::cancelled`], which the wrapped future waits on.
    shared: Arc<Shared>,
    _runtime: PhantomData<fn() -> R>,
}

/// The state of one cancellation, shared by a [`CancelHandle`] and its [`Cancelled`] future.
struct Shared {
    /// Set when the handle cancels or drops.
    cancelled: AtomicBool,
    /// The task of a [`Cancelled`] future that is waiting.
    waiter: AtomicWaker,
}

impl<R> CancelHandle<R>
where R: AsyncRuntime
{
    /// Create a handle that has not been cancelled.
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                cancelled: AtomicBool::new(false),
                waiter: AtomicWaker::new(),
            }),
            _runtime: PhantomData,
        }
    }

    /// A future that completes once this handle cancels or drops.
    ///
    /// The future reads the cancellation flag first in every poll, so a cancellation that
    /// happened before a poll is seen in that poll. One future waits at a time.
    pub(crate) fn cancelled(&self) -> Cancelled {
        Cancelled {
            shared: self.shared.clone(),
        }
    }

    /// Signal cancellation once. Cancelling an already completed future is harmless.
    #[since(version = "0.10.1")]
    pub fn cancel(&mut self) {
        self.shared.cancelled.store(true, Ordering::Release);
        self.shared.waiter.wake();
    }
}

impl<R> Drop for CancelHandle<R>
where R: AsyncRuntime
{
    fn drop(&mut self) {
        self.cancel();
    }
}

/// A future that completes once its [`CancelHandle`] cancels or drops.
pub(crate) struct Cancelled {
    shared: Arc<Shared>,
}

impl Future for Cancelled {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.shared.cancelled.load(Ordering::Acquire) {
            return Poll::Ready(());
        }

        self.shared.waiter.register(cx.waker());

        // Read the flag again: a cancellation between the first read and `register()` found no
        // waker to wake.
        if self.shared.cancelled.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
