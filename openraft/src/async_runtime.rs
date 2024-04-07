//! `async` runtime interface.
//!
//! `async` runtime is an abstraction over different asynchronous runtimes, such as `tokio`,
//! `async-std`, etc.
use std::fmt::Debug;
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

use crate::Instant;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::TokioInstant;

/// A trait defining interfaces with an asynchronous runtime.
///
/// The intention of this trait is to allow an application using this crate to bind an asynchronous
/// runtime that suits it the best.
///
/// Some additional related functions are also exposed by this trait.
///
/// ## Note
///
/// The default asynchronous runtime is `tokio`.
pub trait AsyncRuntime: Debug + Default + PartialEq + Eq + OptionalSend + OptionalSync + 'static {
    /// The error type of [`Self::JoinHandle`].
    type JoinError: Debug + Display + OptionalSend;

    /// The return type of [`Self::spawn`].
    type JoinHandle<T: OptionalSend + 'static>: Future<Output = Result<T, Self::JoinError>>
        + OptionalSend
        + OptionalSync
        + Unpin;

    /// The type that enables the user to sleep in an asynchronous runtime.
    type Sleep: Future<Output = ()> + OptionalSend + OptionalSync;

    /// A measurement of a monotonically non-decreasing clock.
    type Instant: Instant;

    /// The timeout error type.
    type TimeoutError: Debug + Display + OptionalSend;

    /// The timeout type used by [`Self::timeout`] and [`Self::timeout_at`] that enables the user
    /// to await the outcome of a [`Future`].
    type Timeout<R, T: Future<Output = R> + OptionalSend>: Future<Output = Result<R, Self::TimeoutError>> + OptionalSend;

    /// Type of a thread-local random number generator.
    type ThreadLocalRng: rand::Rng;

    /// Type of a `oneshot` sender.
    type OneshotSender<T: OptionalSend>: AsyncOneshotSendExt<T> + OptionalSend + OptionalSync + Debug + Sized;

    /// Type of a `oneshot` receiver error.
    type OneshotReceiverError: std::error::Error + OptionalSend;

    /// Type of a `oneshot` receiver.
    type OneshotReceiver<T: OptionalSend>: OptionalSend
        + OptionalSync
        + Future<Output = Result<T, Self::OneshotReceiverError>>
        + Unpin;

    /// Spawn a new task.
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static;

    /// Wait until `duration` has elapsed.
    fn sleep(duration: Duration) -> Self::Sleep;

    /// Wait until `deadline` is reached.
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep;

    /// Require a [`Future`] to complete before the specified duration has elapsed.
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F>;

    /// Require a [`Future`] to complete before the specified instant in time.
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F>;

    /// Check if the [`Self::JoinError`] is `panic`.
    fn is_panic(join_error: &Self::JoinError) -> bool;

    /// Get the random number generator to use for generating random numbers.
    ///
    /// # Note
    ///
    /// This is a per-thread instance, which cannot be shared across threads or
    /// sent to another thread.
    fn thread_rng() -> Self::ThreadLocalRng;

    /// Creates a new one-shot channel for sending single values.
    ///
    /// The function returns separate "send" and "receive" handles. The `Sender`
    /// handle is used by the producer to send the value. The `Receiver` handle is
    /// used by the consumer to receive the value.
    ///
    /// Each handle can be used on separate tasks.
    fn oneshot<T>() -> (Self::OneshotSender<T>, Self::OneshotReceiver<T>)
    where T: OptionalSend;
}

/// `Tokio` is the default asynchronous executor.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TokioRuntime;

pub struct TokioOneShotSender<T: OptionalSend>(pub tokio::sync::oneshot::Sender<T>);

impl AsyncRuntime for TokioRuntime {
    type JoinError = tokio::task::JoinError;
    type JoinHandle<T: OptionalSend + 'static> = tokio::task::JoinHandle<T>;
    type Sleep = tokio::time::Sleep;
    type Instant = TokioInstant;
    type TimeoutError = tokio::time::error::Elapsed;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = tokio::time::Timeout<T>;
    type ThreadLocalRng = rand::rngs::ThreadRng;
    type OneshotSender<T: OptionalSend> = TokioOneShotSender<T>;
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
        (TokioOneShotSender(tx), rx)
    }
}

pub trait AsyncOneshotSendExt<T> {
    /// Attempts to send a value on this channel, returning it back if it could
    /// not be sent.
    ///
    /// This method consumes `self` as only one value may ever be sent on a `oneshot`
    /// channel. It is not marked async because sending a message to an `oneshot`
    /// channel never requires any form of waiting.  Because of this, the `send`
    /// method can be used in both synchronous and asynchronous code without
    /// problems.
    fn send(self, t: T) -> Result<(), T>;
}

impl<T: OptionalSend> AsyncOneshotSendExt<T> for TokioOneShotSender<T> {
    #[inline]
    fn send(self, t: T) -> Result<(), T> {
        self.0.send(t)
    }
}

impl<T: OptionalSend> Debug for TokioOneShotSender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TokioSendWrapper").finish()
    }
}
