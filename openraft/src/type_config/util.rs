use std::future::Future;
use std::time::Duration;

use futures::Stream;
use openraft_macros::since;

use crate::Instant;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::async_runtime::Mpsc;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::Oneshot;
use crate::async_runtime::mutex::Mutex;
use crate::async_runtime::watch::Watch;
use crate::error::ErrorSource;
use crate::type_config::AsyncRuntime;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::ErrorSourceOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscOf;
use crate::type_config::alias::MpscReceiverOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::MutexOf;
use crate::type_config::alias::OneshotOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SleepOf;
use crate::type_config::alias::TimeoutOf;
use crate::type_config::alias::WatchOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;

/// Collection of utility methods to `RaftTypeConfig` function.
#[since(version = "0.10.0")]
pub trait TypeConfigExt: RaftTypeConfig {
    // Time related methods

    /// Returns the current time.
    #[track_caller]
    fn now() -> InstantOf<Self> {
        InstantOf::<Self>::now()
    }

    /// Wait until `duration` has elapsed.
    #[track_caller]
    fn sleep(duration: Duration) -> SleepOf<Self> {
        AsyncRuntimeOf::<Self>::sleep(duration)
    }

    /// Yield control back to the async runtime, allowing other tasks to run.
    ///
    /// This is useful for long-running computations that want to avoid
    /// starving other tasks. The default implementation sleeps for 1 nanosecond
    /// to ensure the runtime actually yields (some runtimes may optimize
    /// zero-duration sleep as a no-op).
    #[track_caller]
    fn yield_now() -> SleepOf<Self> {
        AsyncRuntimeOf::<Self>::sleep(Duration::from_nanos(1))
    }

    /// Wait until `deadline` is reached.
    #[track_caller]
    fn sleep_until(deadline: InstantOf<Self>) -> SleepOf<Self> {
        AsyncRuntimeOf::<Self>::sleep_until(deadline)
    }

    /// Require a [`Future`] to complete before the specified duration has elapsed.
    #[track_caller]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> TimeoutOf<Self, R, F> {
        AsyncRuntimeOf::<Self>::timeout(duration, future)
    }

    /// Require a [`Future`] to complete before the specified instant in time.
    #[track_caller]
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(
        deadline: InstantOf<Self>,
        future: F,
    ) -> TimeoutOf<Self, R, F> {
        AsyncRuntimeOf::<Self>::timeout_at(deadline, future)
    }

    // Synchronization methods

    /// Creates a new one-shot channel for sending single values.
    ///
    /// This is just a wrapper of
    /// [`AsyncRuntime::Oneshot::channel()`](`crate::async_runtime::Oneshot::channel`).
    #[track_caller]
    fn oneshot<T>() -> (OneshotSenderOf<Self, T>, OneshotReceiverOf<Self, T>)
    where T: OptionalSend {
        OneshotOf::<Self>::channel()
    }

    /// Creates a mpsc channel for communicating between asynchronous
    /// tasks with backpressure.
    ///
    /// This is just a wrapper of
    /// [`AsyncRuntime::Mpsc::channel()`](`crate::async_runtime::Mpsc::channel`).
    #[track_caller]
    fn mpsc<T>(buffer: usize) -> (MpscSenderOf<Self, T>, MpscReceiverOf<Self, T>)
    where T: OptionalSend {
        MpscOf::<Self>::channel(buffer)
    }

    /// Converts an mpsc receiver into a [`Stream`].
    ///
    /// This is useful for passing receiver data to streaming APIs
    /// in a runtime-agnostic way.
    fn mpsc_to_stream<T>(rx: MpscReceiverOf<Self, T>) -> impl Stream<Item = T>
    where T: OptionalSend {
        futures::stream::unfold(rx, |mut rx| async move {
            let item = MpscReceiver::recv(&mut rx).await?;
            Some((item, rx))
        })
    }

    /// Creates a watch channel for watching for changes to a value from multiple
    /// points in the code base.
    ///
    /// This is just a wrapper of
    /// [`AsyncRuntime::Watch::channel()`](`crate::async_runtime::Watch::channel`).
    #[track_caller]
    fn watch_channel<T>(init: T) -> (WatchSenderOf<Self, T>, WatchReceiverOf<Self, T>)
    where T: OptionalSend + OptionalSync {
        WatchOf::<Self>::channel(init)
    }

    /// Creates a Mutex lock.
    ///
    /// This is just a wrapper of
    /// [`AsyncRuntime::Mutex::new()`](`crate::async_runtime::Mutex::new`).
    #[track_caller]
    fn mutex<T>(value: T) -> MutexOf<Self, T>
    where T: OptionalSend {
        MutexOf::<Self, T>::new(value)
    }

    // Task methods

    /// Spawn a new task.
    #[track_caller]
    fn spawn<T>(future: T) -> JoinHandleOf<Self, T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        AsyncRuntimeOf::<Self>::spawn(future)
    }

    /// Create a runtime and run the given future to completion.
    ///
    /// This is a convenience method for testing. It creates a runtime with
    /// default configuration and runs the future on it.
    ///
    /// This runs synchronously on the current thread, so `Send` is not required.
    #[track_caller]
    fn run<F, T>(future: F) -> T
    where
        F: Future<Output = T>,
        T: OptionalSend,
    {
        <AsyncRuntimeOf<Self> as AsyncRuntime>::run(future)
    }

    // Error methods

    /// Create an error source from a string message.
    ///
    /// This is a convenience wrapper for [`ErrorSource::from_string`].
    fn err_from_string(msg: impl ToString) -> ErrorSourceOf<Self> {
        ErrorSourceOf::<Self>::from_string(msg)
    }

    /// Create an error source from an error reference.
    ///
    /// This is a convenience wrapper for [`ErrorSource::from_error`].
    fn err_from_error<E: std::error::Error + 'static>(e: &E) -> ErrorSourceOf<Self> {
        ErrorSourceOf::<Self>::from_error(e)
    }
}

impl<T> TypeConfigExt for T where T: RaftTypeConfig {}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use futures::StreamExt;

    use crate::OptionalSend;
    use crate::RaftTypeConfig;
    use crate::async_runtime::MpscSender;
    use crate::impls::TokioRuntime;
    use crate::type_config::TypeConfigExt;

    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
    pub(crate) struct UTConfig {}

    impl RaftTypeConfig for UTConfig {
        type D = u64;
        type R = ();
        type NodeId = u64;
        type Node = ();
        type Term = u64;
        type LeaderId = crate::impls::leader_id_adv::LeaderId<Self>;
        type Vote = crate::impls::Vote<Self>;
        type Entry = crate::Entry<Self>;
        type SnapshotData = Cursor<Vec<u8>>;
        type AsyncRuntime = TokioRuntime;
        type Responder<T>
            = crate::impls::OneshotResponder<Self, T>
        where T: OptionalSend + 'static;
        type ErrorSource = anyerror::AnyError;
    }

    #[test]
    fn test_mpsc_to_stream() {
        UTConfig::run(async {
            let (tx, rx) = UTConfig::mpsc::<u64>(16);
            let stream = UTConfig::mpsc_to_stream(rx);
            futures::pin_mut!(stream);

            // Send items
            tx.send(1).await.unwrap();
            tx.send(2).await.unwrap();
            tx.send(3).await.unwrap();
            drop(tx); // Close sender

            // Receive from stream
            let mut received = vec![];
            while let Some(item) = stream.next().await {
                received.push(item);
            }

            assert_eq!(received, vec![1, 2, 3]);
        });
    }

    #[test]
    fn test_mpsc_to_stream_empty() {
        UTConfig::run(async {
            let (tx, rx) = UTConfig::mpsc::<u64>(16);
            let stream = UTConfig::mpsc_to_stream(rx);
            futures::pin_mut!(stream);

            // Close sender immediately
            drop(tx);

            // Stream should be empty
            let item = stream.next().await;
            assert!(item.is_none());
        });
    }

    /// Ensure the returned stream is 'static
    fn _assert_static<T: 'static>(_: T) {}

    #[test]
    fn test_mpsc_to_stream_is_static() {
        let (_, rx) = UTConfig::mpsc::<u64>(16);
        let stream = UTConfig::mpsc_to_stream(rx);
        _assert_static(stream);
    }
}
