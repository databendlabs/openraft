use std::fmt::Debug;
use std::fmt::Display;
use std::future::Future;
use std::time::Duration;

use crate::Instant;
use crate::OptionalSend;
use crate::OptionalSync;

#[cfg(not(feature = "monoio"))] pub mod tokio;

#[cfg(feature = "monoio")] pub mod monoio;

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

pub trait AsyncOneshotSendExt<T>: Unpin {
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
