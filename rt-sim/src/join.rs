//! Join handles for spawned tasks, and the wrapper that catches a task's panic.

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use pin_project_lite::pin_project;
use tokio::sync::oneshot;

/// Why a spawned task did not produce a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimJoinError {
    /// The task panicked; the payload's message, if it was a string.
    Panic(String),
    /// The task was dropped before returning a result.
    Cancelled,
}

impl fmt::Display for SimJoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimJoinError::Panic(message) => write!(f, "task panicked: {message}"),
            SimJoinError::Cancelled => write!(f, "task cancelled: no result received"),
        }
    }
}

impl std::error::Error for SimJoinError {}

/// Resolves to the spawned task's output. Dropping it detaches the task; the task keeps running.
///
/// Returns [`SimJoinError::Cancelled`] if the runtime drops the task before it produces a result.
pub struct SimJoinHandle<T> {
    receiver: oneshot::Receiver<Result<T, SimJoinError>>,
}

impl<T> SimJoinHandle<T> {
    pub(crate) fn new(receiver: oneshot::Receiver<Result<T, SimJoinError>>) -> Self {
        SimJoinHandle { receiver }
    }
}

impl<T> Future for SimJoinHandle<T> {
    type Output = Result<T, SimJoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let receiver = Pin::new(&mut this.receiver);
        let polled = receiver.poll(cx);
        match polled {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(_)) => Poll::Ready(Err(SimJoinError::Cancelled)),
            Poll::Pending => Poll::Pending,
        }
    }
}

pin_project! {
    /// A spawned future and the sender its outcome goes to. A panic while polling it becomes
    /// [`SimJoinError::Panic`] instead of unwinding through the executor.
    pub(crate) struct Task<F: Future> {
        #[pin]
        future: F,
        sender: Option<oneshot::Sender<Result<F::Output, SimJoinError>>>,
    }
}

impl<F> Task<F>
where F: Future
{
    pub(crate) fn new(future: F, sender: oneshot::Sender<Result<F::Output, SimJoinError>>) -> Self {
        Task {
            future,
            sender: Some(sender),
        }
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
        let sender = this.sender.take().expect("rt-sim: task polled after completion");
        // A dropped receiver means the task was detached.
        let _ = sender.send(result);
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

#[cfg(test)]
mod tests {
    use std::task::Waker;
    use std::time::Duration;

    use openraft_rt::AsyncRuntime;
    use tokio::sync::oneshot;

    use super::*;
    use crate::SimRuntime;

    #[test]
    fn runtime_drop_cancels_pending_task() {
        let mut rt = SimRuntime::with_seed(0);
        let mut captured = None;
        rt.block_on(async {
            let future = std::future::pending::<u64>();
            let handle = SimRuntime::spawn(future);
            captured = Some(handle);
        });
        drop(rt);
        let mut handle = captured.unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        let handle = Pin::new(&mut handle);
        let polled = handle.poll(&mut cx);
        let expected = Poll::Ready(Err(SimJoinError::Cancelled));
        assert_eq!(polled, expected);
    }

    #[test]
    fn runtime_drop_keeps_completed_output() {
        let mut rt = SimRuntime::with_seed(0);
        let mut captured = None;
        rt.block_on(async {
            let future = async { 7_u64 };
            let handle = SimRuntime::spawn(future);
            let delay = Duration::from_millis(1);
            SimRuntime::sleep(delay).await;
            captured = Some(handle);
        });
        drop(rt);
        let mut handle = captured.unwrap();
        let mut cx = Context::from_waker(Waker::noop());
        let handle = Pin::new(&mut handle);
        let polled = handle.poll(&mut cx);
        let expected = Poll::Ready(Ok(7));
        assert_eq!(polled, expected);
    }

    #[test]
    fn dropping_join_handle_detaches_task() {
        let mut rt = SimRuntime::with_seed(0);
        let output = rt.block_on(async {
            let (sender, receiver) = oneshot::channel();
            let future = async move {
                let sent = sender.send(42_u64);
                sent.unwrap();
            };
            let handle = SimRuntime::spawn(future);
            drop(handle);
            let result = receiver.await;
            result.unwrap()
        });
        assert_eq!(output, 42);
    }

    #[test]
    fn cancelled_error_reports_missing_result() {
        let error = SimJoinError::Cancelled;
        let is_panic = SimRuntime::is_panic(&error);
        assert!(!is_panic);
        let message = error.to_string();
        assert_eq!(message, "task cancelled: no result received");
    }
}
