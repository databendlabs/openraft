//! `async` runtime interface.
//!
//! `async` runtime is an abstraction over different asynchronous runtimes, such as `tokio`,
//! `async-std`, etc.

use std::any::Any;
use std::fmt::Debug;
use std::fmt::Display;
use std::future::Future;
use std::io;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures_util::FutureExt;
use openraft_macros::since;

use crate::CancelHandle;
use crate::Instant;
use crate::Mpsc;
use crate::MpscReceiver;
use crate::Mutex;
use crate::Oneshot;
use crate::OptionalSend;
use crate::Select2;
use crate::Select3;
use crate::Select4;
use crate::TryRecvError;
use crate::Watch;
use crate::select2_future::Select2Future;

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
#[since(version = "0.10.1", change = "add cancellable futures")]
#[since(
    version = "0.10.0",
    change = "remove the `OptionalSend` and `OptionalSync` requirements"
)]
#[since(version = "0.10.0", change = "add biased select2, select3, and select4 methods")]
pub trait AsyncRuntime: Debug + 'static {
    /// The error type of [`Self::JoinHandle`].
    type JoinError: Debug + Display + OptionalSend;

    /// The return type of [`Self::spawn`].
    #[since(version = "0.10.0", change = "remove the `OptionalSync` requirement")]
    type JoinHandle<T: OptionalSend + 'static>: Future<Output = Result<T, Self::JoinError>> + OptionalSend + Unpin;

    /// The type that enables the user to sleep in an asynchronous runtime.
    #[since(version = "0.10.0", change = "remove the `OptionalSync` requirement")]
    type Sleep: Future<Output = ()> + OptionalSend;

    /// A measurement of a monotonically non-decreasing clock.
    type Instant: Instant;

    /// The timeout error type.
    type TimeoutError: Debug + Display + OptionalSend;

    /// The timeout type used by [`Self::timeout`] and [`Self::timeout_at`] that enables the user
    /// to await the outcome of a [`Future`].
    type Timeout<R, T: Future<Output = R> + OptionalSend>: Future<Output = Result<R, Self::TimeoutError>> + OptionalSend;

    /// Type of the random number generator that [`Self::thread_rng`] returns.
    ///
    /// Each runtime supplies its own. Tokio, Compio and Monoio return `rand`'s `ThreadRng`, which
    /// is bound to the calling thread, while
    /// [`DeterministicRng`](crate::deterministic_rng::DeterministicRng) and the simulator return
    /// an owned, seeded `SmallRng`. Generic code that moves the generator to another thread needs
    /// a bound such as `Self::ThreadLocalRng: Send`.
    type ThreadLocalRng: rand::Rng;

    /// Spawn a new task.
    #[track_caller]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static;

    /// Wait until `duration` has elapsed since this call.
    ///
    /// The deadline is taken when `sleep` is called, as `tokio::time::sleep` does, not when the
    /// returned future is first polled: a sleep first polled after `duration` has passed is ready
    /// at once.
    #[track_caller]
    fn sleep(duration: Duration) -> Self::Sleep;

    /// Wait until `deadline` is reached.
    #[track_caller]
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep;

    /// Require a [`Future`] to complete before `duration` has elapsed since this call.
    ///
    /// The deadline is taken when `timeout` is called, as for [`Self::sleep`]. Each poll polls
    /// `future` before it checks the deadline, as `tokio::time::timeout` does, so a `future` that
    /// is ready when the deadline has passed still completes with its output.
    #[track_caller]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F>;

    /// Require a [`Future`] to complete before the specified instant in time.
    ///
    /// As with [`Self::timeout`], each poll polls `future` before it checks `deadline`.
    #[track_caller]
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F>;

    /// Wait for one of two futures to complete, returning its output.
    ///
    /// Futures are polled in argument order on every poll. The first ready future wins;
    /// later futures are not polled once a ready future is found. A continuously ready
    /// earlier future can starve later futures when selection is repeated in a loop.
    ///
    /// Owned futures are dropped when selection completes or is cancelled. Pass `&mut`
    /// references to `Unpin` futures, or pinned references, to retain losing futures.
    /// Cancellation safety depends on the futures being selected.
    ///
    /// # Example
    ///
    /// ```
    /// use openraft_rt::{AsyncRuntime, Select2};
    ///
    /// async fn choose<Rt>()
    /// where Rt: AsyncRuntime
    /// {
    ///     let selected = Rt::select2(async { 1_u8 }, async { "two" }).await;
    ///     let expected = Select2::First(1);
    ///     assert_eq!(selected, expected);
    /// }
    /// ```
    #[since(version = "0.10.0")]
    fn select2<A, B>(a: A, b: B) -> impl Future<Output = Select2<A::Output, B::Output>> + OptionalSend
    where
        A: Future + OptionalSend,
        B: Future + OptionalSend,
    {
        Select2Future::new(a, b)
    }

    /// Wait for one of three futures to complete, returning its output.
    ///
    /// Uses the same argument-order bias and cancellation rules as [`Self::select2`].
    /// The default implementation composes [`Self::select2`], preserving runtime overrides.
    #[since(version = "0.10.0")]
    fn select3<A, B, C>(
        a: A,
        b: B,
        c: C,
    ) -> impl Future<Output = Select3<A::Output, B::Output, C::Output>> + OptionalSend
    where
        A: Future + OptionalSend,
        B: Future + OptionalSend,
        C: Future + OptionalSend,
    {
        // An `async` block would reserve the bytes of `a`, `b` and `c` twice: once for the captured
        // variables and once in the select future it polls (rust-lang/rust#62958). `map()` stores
        // them once.
        let tail = Self::select2(b, c);
        Self::select2(a, tail).map(|selected| match selected {
            Select2::First(value) => Select3::First(value),
            Select2::Second(Select2::First(value)) => Select3::Second(value),
            Select2::Second(Select2::Second(value)) => Select3::Third(value),
        })
    }

    /// Wait for one of four futures to complete, returning its output.
    ///
    /// Uses the same argument-order bias and cancellation rules as [`Self::select2`].
    /// The default implementation composes [`Self::select2`], preserving runtime overrides.
    #[since(version = "0.10.0")]
    fn select4<A, B, C, D>(
        a: A,
        b: B,
        c: C,
        d: D,
    ) -> impl Future<Output = Select4<A::Output, B::Output, C::Output, D::Output>> + OptionalSend
    where
        A: Future + OptionalSend,
        B: Future + OptionalSend,
        C: Future + OptionalSend,
        D: Future + OptionalSend,
    {
        let head = Self::select2(a, b);
        let tail = Self::select2(c, d);
        Self::select2(head, tail).map(|selected| match selected {
            Select2::First(Select2::First(value)) => Select4::First(value),
            Select2::First(Select2::Second(value)) => Select4::Second(value),
            Select2::Second(Select2::First(value)) => Select4::Third(value),
            Select2::Second(Select2::Second(value)) => Select4::Fourth(value),
        })
    }

    /// Check if the [`Self::JoinError`] is `panic`.
    #[track_caller]
    fn is_panic(join_error: &Self::JoinError) -> bool;

    /// Get the random number generator to use for generating random numbers.
    ///
    /// # Note
    ///
    /// Whether the generator can move to another thread depends on the runtime: see
    /// [`Self::ThreadLocalRng`].
    #[track_caller]
    fn thread_rng() -> Self::ThreadLocalRng;

    /// The bounded MPSC channel implementation.
    type Mpsc: Mpsc;

    /// The watch channel implementation.
    type Watch: Watch;

    /// The oneshot channel implementation.
    type Oneshot: Oneshot;

    /// The async mutex implementation.
    type Mutex<T: OptionalSend + 'static>: Mutex<T>;

    /// Create a new runtime instance for testing purposes.
    ///
    /// **Note**: This method is primarily intended for testing and is not used by Openraft
    /// internally. In production applications, the runtime should be created and managed
    /// by the application itself, with Openraft running within that runtime.
    ///
    /// # Arguments
    ///
    /// * `threads` - Number of worker threads. Multi-threaded runtimes (like Tokio) will use this
    ///   value; single-threaded runtimes (like Monoio, Compio) may ignore it.
    fn new(threads: usize) -> Self;

    /// Run a future to completion on this runtime.
    ///
    /// This runs synchronously on the current thread, so `Send` is not required.
    #[since(version = "0.10.0", change = "allow results without `OptionalSend`")]
    fn block_on<F, T>(&mut self, future: F) -> T
    where F: Future<Output = T>;

    /// Convenience method: create a runtime and run the future to completion.
    ///
    /// Creates a runtime with default configuration (8 threads) and runs the future.
    /// For simple cases where you don't need to reuse the runtime.
    /// If you need to run multiple futures, consider using [`Self::new`] and
    /// [`Self::block_on`] directly.
    ///
    /// This runs synchronously on the current thread, so `Send` is not required.
    #[since(version = "0.10.0", change = "allow results without `OptionalSend`")]
    fn run<F, T>(future: F) -> T
    where
        Self: Sized,
        F: Future<Output = T>,
    {
        Self::new(8).block_on(future)
    }

    /// Run a blocking function on a separate thread.
    ///
    /// The default implementation spawns a new OS thread for each call. When the thread cannot
    /// be created, the returned future resolves to the error of the operating system. When `f`
    /// panics, it resolves to an error that reports the panic and its message.
    /// Runtime implementations may override this with their own thread pool
    /// (e.g., tokio's `spawn_blocking`) for better resource management.
    #[since(version = "0.10.2", change = "the default returns a thread-creation error")]
    #[since(version = "0.10.2", change = "the default reports a panic of `f` as an error")]
    fn spawn_blocking<F, T>(f: F) -> impl Future<Output = Result<T, io::Error>> + Send
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = futures_channel::oneshot::channel();
        let spawned = std::thread::Builder::new().spawn(move || {
            tx.send(std::panic::catch_unwind(AssertUnwindSafe(f))).ok();
        });
        blocking_output(spawned.map(drop), rx)
    }

    /// Try to poll a value from the channel within the given deadline.
    ///
    /// By default, this first checks synchronously if a value is already available.
    /// If not, it uses the AsyncRuntime timeout mechanism to wait until:
    /// - a new element arrives
    /// - the deadline is reached
    #[since(version = "0.10.0")]
    fn mpsc_recv_deadline<T: OptionalSend>(
        receiver: &mut <Self::Mpsc as Mpsc>::Receiver<T>,
        deadline: Self::Instant,
    ) -> impl Future<Output = Result<T, TryRecvError>> + OptionalSend {
        async move {
            match receiver.try_recv() {
                Ok(value) => Ok(value),
                Err(TryRecvError::Disconnected) => Err(TryRecvError::Disconnected),
                Err(TryRecvError::Empty) if <Self::Instant as Instant>::now() >= deadline => Err(TryRecvError::Empty),
                Err(TryRecvError::Empty) => match Self::timeout_at(deadline, receiver.recv()).await {
                    Ok(None) => Err(TryRecvError::Disconnected),
                    Ok(Some(value)) => Ok(value),
                    Err(_) => Err(TryRecvError::Empty),
                },
            }
        }
    }

    /// Wrap a future with a cancellation handle.
    ///
    /// The returned future yields `Some(output)` when the supplied future completes, or `None`
    /// when the handle is signalled or dropped. Cancellation is polled first when both are ready,
    /// and a cancellation that happened before a poll keeps that poll from polling the supplied
    /// future.
    /// The caller polls or spawns the returned future; borrowed futures are supported.
    /// Use [`Self::cancellable_by`] when a cancellation future is already available.
    #[since(version = "0.10.2", change = "cancelling before a poll keeps the work unpolled")]
    #[since(version = "0.10.1")]
    fn cancellable<F>(
        future: F,
    ) -> (
        CancelHandle<Self>,
        impl Future<Output = Option<F::Output>> + OptionalSend,
    )
    where
        Self: Sized,
        F: Future + OptionalSend,
    {
        let handle = CancelHandle::new();
        let future = Self::cancellable_by(future, handle.cancelled());
        (handle, future)
    }

    /// Wrap work with a future that signals cancellation.
    ///
    /// Return `Some(output)` when the work completes, or `None` when the cancel future completes.
    /// Cancellation wins when both are ready. See [`Self::select2`] for ownership rules.
    #[since(version = "0.10.1")]
    fn cancellable_by<F, Cancel>(future: F, cancel: Cancel) -> impl Future<Output = Option<F::Output>> + OptionalSend
    where
        F: Future + OptionalSend,
        Cancel: Future + OptionalSend,
    {
        Self::select2(cancel, future).map(|selected| match selected {
            Select2::First(_) => None,
            Select2::Second(output) => Some(output),
        })
    }
}

/// The output of a blocking worker: the error that kept its thread from starting, or else what
/// the worker sends through `rx`.
async fn blocking_output<T>(
    spawned: io::Result<()>,
    rx: futures_channel::oneshot::Receiver<std::thread::Result<T>>,
) -> Result<T, io::Error> {
    spawned?;
    match rx.await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(payload)) => Err(io::Error::other(format!(
            "spawn_blocking worker panicked: {}",
            panic_message(payload.as_ref())
        ))),
        Err(_) => Err(io::Error::other("spawn_blocking worker sent no result")),
    }
}

/// The message of a panic payload: the payload of `panic!` is a `&'static str` or a `String`.
fn panic_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(msg) = payload.downcast_ref::<&'static str>() {
        msg
    } else if let Some(msg) = payload.downcast_ref::<String>() {
        msg
    } else {
        "the panic payload is not a string"
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use futures_util::FutureExt;

    use super::blocking_output;

    /// A thread-creation error is returned at once, without waiting for a worker result.
    #[test]
    fn test_blocking_output_returns_the_spawn_error() {
        let (_tx, rx) = futures_channel::oneshot::channel::<std::thread::Result<u32>>();
        let spawned = Err(io::Error::from(io::ErrorKind::WouldBlock));

        let got = blocking_output(spawned, rx).now_or_never();

        let err = got.expect("the spawn error should not wait for the worker").unwrap_err();
        assert_eq!(io::ErrorKind::WouldBlock, err.kind());
    }
}
