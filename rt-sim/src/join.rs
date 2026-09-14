//! Join handles for spawned tasks, and the wrapper that catches a task's panic.

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use pin_project_lite::pin_project;

/// Why a spawned task did not produce a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimJoinError {
    /// The task panicked; the payload's message, if it was a string.
    Panic(String),
}

impl fmt::Display for SimJoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimJoinError::Panic(message) => write!(f, "task panicked: {message}"),
        }
    }
}

impl std::error::Error for SimJoinError {}

struct Slot<T> {
    result: Option<Result<T, SimJoinError>>,
    waker: Option<Waker>,
}

/// The shared cell a task writes its outcome into.
pub(crate) struct JoinSlot<T> {
    inner: Mutex<Slot<T>>,
}

impl<T> JoinSlot<T> {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(JoinSlot {
            inner: Mutex::new(Slot {
                result: None,
                waker: None,
            }),
        })
    }

    fn complete(&self, result: Result<T, SimJoinError>) {
        let waker = {
            let mut slot = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            slot.result = Some(result);
            slot.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// Resolves to the spawned task's output. Dropping it detaches the task; the task keeps running.
pub struct SimJoinHandle<T> {
    slot: Arc<JoinSlot<T>>,
}

impl<T> SimJoinHandle<T> {
    pub(crate) fn new(slot: Arc<JoinSlot<T>>) -> Self {
        SimJoinHandle { slot }
    }
}

impl<T> Future for SimJoinHandle<T> {
    type Output = Result<T, SimJoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = self.slot.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match slot.result.take() {
            Some(result) => Poll::Ready(result),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

pin_project! {
    /// A spawned future and the slot its outcome goes to. A panic while polling it becomes
    /// [`SimJoinError::Panic`] instead of unwinding through the executor.
    pub(crate) struct Task<F: Future> {
        #[pin]
        future: F,
        slot: Arc<JoinSlot<F::Output>>,
    }
}

impl<F> Task<F>
where F: Future
{
    pub(crate) fn new(future: F, slot: Arc<JoinSlot<F::Output>>) -> Self {
        Task { future, slot }
    }
}

impl<F> Future for Task<F>
where F: Future
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.project();
        let mut future = this.future;
        let polled = std::panic::catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx)));
        let result = match polled {
            Ok(Poll::Pending) => return Poll::Pending,
            Ok(Poll::Ready(output)) => Ok(output),
            Err(payload) => Err(SimJoinError::Panic(panic_message(payload.as_ref()))),
        };
        this.slot.complete(result);
        Poll::Ready(())
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}
