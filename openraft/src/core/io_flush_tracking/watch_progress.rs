use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::Vote;
use crate::async_runtime::watch::RecvError;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::io_flush_tracking::FlushPoint;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::WatchReceiverOf;

/// Handle for tracking log I/O flush progress.
///
/// The value is `None` until the node starts running, in [`Raft::new`], or in [`Raft::run`] after
/// [`Raft::build`]. It then publishes the flush point it recovered from storage, which holds no log
/// id: the stored vote, or the default vote when storage holds none. So `Some` alone does not mean
/// that the node wrote anything to storage. Later values follow the votes and logs that storage
/// reports as flushed.
///
/// [`Raft::new`]: crate::Raft::new
/// [`Raft::run`]: crate::Raft::run
/// [`Raft::build`]: crate::Raft::build
pub type LogProgress<C> = WatchProgress<C, Option<FlushPoint<C>>>;

/// Handle for tracking vote I/O flush progress.
///
/// The value is `None` until the node starts running, as for [`LogProgress`]. It then publishes
/// the vote it recovered from storage, or the default vote when storage holds none, so `Some`
/// alone does not mean that a vote was saved. Later values are the votes that storage reports as
/// flushed.
pub type VoteProgress<C> = WatchProgress<C, Option<Vote<LeaderIdOf<C>>>>;

/// Handle for tracking commit log progress.
///
/// Returns `None` if no log has been committed yet.
/// Returns `Some(LogId)` containing the latest committed log id.
pub type CommitProgress<C> = WatchProgress<C, Option<LogIdOf<C>>>;

/// Handle for tracking snapshot persistence progress.
///
/// Returns `None` if no snapshot has been persisted yet.
/// Returns `Some(LogId)` containing the last persisted snapshot log id.
pub type SnapshotProgress<C> = WatchProgress<C, Option<LogIdOf<C>>>;

/// Handle for tracking applied log progress.
///
/// Returns `None` if no log has been applied yet.
/// Returns `Some(LogId)` containing the last applied log id.
pub type AppliedProgress<C> = WatchProgress<C, Option<LogIdOf<C>>>;

/// Watch handle for tracking I/O flush progress.
///
/// Provides five operations:
/// - [`get()`](Self::get): Get current progress state immediately
/// - [`wait_until_ge()`](Self::wait_until_ge): Wait asynchronously until progress reaches a
///   threshold
/// - [`wait_until()`](Self::wait_until): Wait asynchronously until progress satisfies a custom
///   condition
/// - [`changed()`](Self::changed): Wait for any value change notification
/// - [`next()`](Self::next): Wait for and return the next changed value
///
/// # Concurrency
///
/// - Multiple handles can watch concurrently (each clones the receiver)
/// - `get()` provides a snapshot at call time (may be stale immediately)
/// - `wait_until_ge()` is sequentially consistent: if it returns, all future `get()` calls will see
///   a value >= the returned value
///
/// This is a thin wrapper around a watch channel receiver that enforces the progress
/// tracking semantics (values must be comparable via `PartialOrd`).
#[derive(Clone)]
pub struct WatchProgress<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend + OptionalSync + PartialOrd + Clone,
{
    inner: WatchReceiverOf<C, T>,
}

impl<C, T> WatchProgress<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend + OptionalSync + PartialOrd + Clone,
{
    pub(crate) fn new(inner: WatchReceiverOf<C, T>) -> Self {
        Self { inner }
    }

    /// Wait until the flushed I/O progress becomes greater than or equal to the target value.
    ///
    /// Returns the current progress state once the condition is satisfied. If the progress
    /// is already >= `target`, returns immediately.
    ///
    /// The wait does not mark the returned state as seen: if this handle had not seen that state
    /// when the wait began, a following [`next()`](Self::next) or [`changed()`](Self::changed)
    /// returns at once for the same state.
    ///
    /// # Errors
    ///
    /// Returns `RecvError` if the sender is dropped (node is shutting down).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let target = Some(FlushPoint::new(Vote::new(2, node_id), Some(log_id(2, node_id, 100))));
    /// let state = log_progress.wait_until_ge(&target).await?;
    /// // state is guaranteed to be >= target
    /// ```
    pub async fn wait_until_ge(&mut self, target: &T) -> Result<T, RecvError> {
        self.inner.wait_until_ge(target).await
    }

    /// Wait until the flushed I/O progress satisfies the given condition.
    ///
    /// Returns the current progress state once the condition is satisfied. If the progress
    /// already satisfies the condition, returns immediately.
    ///
    /// The wait does not mark the returned state as seen: if this handle had not seen that state
    /// when the wait began, a following [`next()`](Self::next) or [`changed()`](Self::changed)
    /// returns at once for the same state. `condition` can be called more than once for the same
    /// state.
    ///
    /// # Errors
    ///
    /// Returns `RecvError` if the sender is dropped (node is shutting down).
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Wait until vote term is exactly 5
    /// let state = vote_progress.wait_until(|v| v.as_ref().map_or(false, |vote| vote.leader_id().term == 5)).await?;
    /// ```
    pub async fn wait_until<F>(&mut self, condition: F) -> Result<T, RecvError>
    where F: Fn(&T) -> bool + OptionalSend {
        self.inner.wait_until(condition).await
    }

    /// Get the current flushed I/O progress state immediately without waiting.
    ///
    /// This returns a snapshot of the most recent flushed I/O operation. The value may become
    /// stale immediately after reading as new I/O operations complete concurrently.
    pub fn get(&self) -> T {
        self.inner.borrow_watched().clone()
    }

    /// Wait for a value change notification.
    ///
    /// Waits until the value changes from the currently observed value. If a new value exists
    /// that hasn't been observed yet, returns immediately. Otherwise, sleeps until a new value
    /// is sent or the sender is dropped.
    ///
    /// After this returns successfully, the new value is marked as seen. Use [`get()`](Self::get)
    /// to retrieve the value.
    ///
    /// # Errors
    ///
    /// Returns `RecvError` if the sender is dropped (node is shutting down).
    ///
    /// # Example
    ///
    /// ```ignore
    /// loop {
    ///     log_progress.changed().await?;
    ///     let current = log_progress.get();
    ///     println!("Progress changed to: {:?}", current);
    /// }
    /// ```
    pub async fn changed(&mut self) -> Result<(), RecvError> {
        self.inner.changed().await
    }

    /// Wait for a change notification, then return the latest value and mark it seen.
    ///
    /// Values sent since the last one this handle has seen are coalesced: `next()` returns the
    /// latest of them, and the following call waits for a value sent after it. A value sent
    /// between the notification and the read is returned once, not again by the following call.
    ///
    /// # Errors
    ///
    /// Returns `RecvError` if the sender is dropped (node is shutting down).
    ///
    /// # Example
    ///
    /// ```ignore
    /// loop {
    ///     let current = log_progress.next().await?;
    ///     println!("Progress changed to: {:?}", current);
    /// }
    /// ```
    pub async fn next(&mut self) -> Result<T, RecvError> {
        self.changed().await?;
        let latest = self.inner.borrow_and_update().clone();
        Ok(latest)
    }
}

#[cfg(test)]
mod watch_progress_test;
