//! `async` runtime interface.
//!
//! `async` runtime is an abstraction over different asynchronous runtimes, such as `tokio`,
//! `async-std`, etc.

use std::fmt::Debug;
use std::fmt::Display;
use std::future::Future;
use std::future::poll_fn;
use std::io;
use std::pin::pin;
use std::task::Poll;
use std::time::Duration;

use openraft_macros::since;

use crate::Instant;
use crate::Mpsc;
use crate::MpscReceiver;
use crate::Mutex;
use crate::Oneshot;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::Select2;
use crate::Select3;
use crate::Select4;
use crate::TryRecvError;
use crate::Watch;

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
#[since(version = "0.10.0", change = "add biased select2, select3, and select4 methods")]
pub trait AsyncRuntime: Debug + OptionalSend + OptionalSync + 'static {
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

    /// Type of thread-local random number generator.
    type ThreadLocalRng: rand::Rng;

    /// Spawn a new task.
    #[track_caller]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static;

    /// Wait until `duration` has elapsed.
    #[track_caller]
    fn sleep(duration: Duration) -> Self::Sleep;

    /// Wait until `deadline` is reached.
    #[track_caller]
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep;

    /// Require a [`Future`] to complete before the specified duration has elapsed.
    #[track_caller]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F>;

    /// Require a [`Future`] to complete before the specified instant in time.
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
        async move {
            let mut a = pin!(a);
            let mut b = pin!(b);
            let select = poll_fn(move |cx| {
                let first = a.as_mut().poll(cx);
                if let Poll::Ready(value) = first {
                    let selected = Select2::First(value);
                    return Poll::Ready(selected);
                }
                let second = b.as_mut().poll(cx);
                if let Poll::Ready(value) = second {
                    let selected = Select2::Second(value);
                    return Poll::Ready(selected);
                }
                Poll::Pending
            });
            select.await
        }
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
        async move {
            let tail = Self::select2(b, c);
            let selected = Self::select2(a, tail).await;
            match selected {
                Select2::First(value) => Select3::First(value),
                Select2::Second(Select2::First(value)) => Select3::Second(value),
                Select2::Second(Select2::Second(value)) => Select3::Third(value),
            }
        }
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
        async move {
            let head = Self::select2(a, b);
            let tail = Self::select2(c, d);
            let selected = Self::select2(head, tail).await;
            match selected {
                Select2::First(Select2::First(value)) => Select4::First(value),
                Select2::First(Select2::Second(value)) => Select4::Second(value),
                Select2::Second(Select2::First(value)) => Select4::Third(value),
                Select2::Second(Select2::Second(value)) => Select4::Fourth(value),
            }
        }
    }

    /// Check if the [`Self::JoinError`] is `panic`.
    #[track_caller]
    fn is_panic(join_error: &Self::JoinError) -> bool;

    /// Get the random number generator to use for generating random numbers.
    ///
    /// # Note
    ///
    /// This is a per-thread instance, which cannot be shared across threads or
    /// sent to another thread.
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
    fn block_on<F, T>(&mut self, future: F) -> T
    where
        F: Future<Output = T>,
        T: OptionalSend;

    /// Convenience method: create a runtime and run the future to completion.
    ///
    /// Creates a runtime with default configuration (8 threads) and runs the future.
    /// For simple cases where you don't need to reuse the runtime.
    /// If you need to run multiple futures, consider using [`Self::new`] and
    /// [`Self::block_on`] directly.
    ///
    /// This runs synchronously on the current thread, so `Send` is not required.
    fn run<F, T>(future: F) -> T
    where
        Self: Sized,
        F: Future<Output = T>,
        T: OptionalSend,
    {
        Self::new(8).block_on(future)
    }

    /// Run a blocking function on a separate thread.
    ///
    /// The default implementation spawns a new OS thread for each call.
    /// Runtime implementations may override this with their own thread pool
    /// (e.g., tokio's `spawn_blocking`) for better resource management.
    fn spawn_blocking<F, T>(f: F) -> impl Future<Output = Result<T, io::Error>> + Send
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = futures_channel::oneshot::channel();
        std::thread::spawn(move || {
            tx.send(f()).ok();
        });
        async { rx.await.map_err(|_| io::Error::other("spawn_blocking task cancelled")) }
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
}
