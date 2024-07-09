use std::future::Future;
use std::time::Duration;

use openraft_macros::since;

use crate::async_runtime::watch::Watch;
use crate::async_runtime::MpscUnbounded;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscUnboundedOf;
use crate::type_config::alias::MpscUnboundedReceiverOf;
use crate::type_config::alias::MpscUnboundedSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SleepOf;
use crate::type_config::alias::TimeoutOf;
use crate::type_config::alias::WatchOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;
use crate::type_config::AsyncRuntime;
use crate::Instant;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;

/// Collection of utility methods to `RaftTypeConfig` function.
#[since(version = "0.10.0")]
pub trait TypeConfigExt: RaftTypeConfig {
    // Time related methods

    /// Returns the current time.
    fn now() -> InstantOf<Self> {
        InstantOf::<Self>::now()
    }

    /// Wait until `duration` has elapsed.
    fn sleep(duration: Duration) -> SleepOf<Self> {
        AsyncRuntimeOf::<Self>::sleep(duration)
    }

    /// Wait until `deadline` is reached.
    fn sleep_until(deadline: InstantOf<Self>) -> SleepOf<Self> {
        AsyncRuntimeOf::<Self>::sleep_until(deadline)
    }

    /// Require a [`Future`] to complete before the specified duration has elapsed.
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> TimeoutOf<Self, R, F> {
        AsyncRuntimeOf::<Self>::timeout(duration, future)
    }

    /// Require a [`Future`] to complete before the specified instant in time.
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
    /// [`AsyncRuntime::oneshot`](`crate::async_runtime::AsyncRuntime::oneshot`).
    fn oneshot<T>() -> (OneshotSenderOf<Self, T>, OneshotReceiverOf<Self, T>)
    where T: OptionalSend {
        AsyncRuntimeOf::<Self>::oneshot()
    }

    /// Creates an unbounded mpsc channel for communicating between asynchronous
    /// tasks without backpressure.
    ///
    /// This is just a wrapper of
    /// [`AsyncRuntime::MpscUnbounded::channel()`](`crate::async_runtime::MpscUnbounded::channel`).
    fn mpsc_unbounded<T>() -> (MpscUnboundedSenderOf<Self, T>, MpscUnboundedReceiverOf<Self, T>)
    where T: OptionalSend {
        MpscUnboundedOf::<Self>::channel()
    }

    /// Creates an watch channel for watching for changes to a value from multiple
    /// points in the code base.
    ///
    /// This is just a wrapper of
    /// [`AsyncRuntime::Watch::channel()`](`crate::async_runtime::Watch::channel`).
    fn watch_channel<T>(init: T) -> (WatchSenderOf<Self, T>, WatchReceiverOf<Self, T>)
    where T: OptionalSend + OptionalSync {
        WatchOf::<Self>::channel(init)
    }

    // Task methods

    /// Spawn a new task.
    fn spawn<T>(future: T) -> JoinHandleOf<Self, T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        AsyncRuntimeOf::<Self>::spawn(future)
    }
}

impl<T> TypeConfigExt for T where T: RaftTypeConfig {}
