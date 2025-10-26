//! I/O flush progress tracking.
//!
//! This module provides watch-based notification channels for tracking when Raft I/O operations
//! (vote saves and log appends) are flushed to storage. It enables applications to:
//!
//! - Wait for specific log entries to be durably written
//! - Track vote changes across leader elections
//! - Ensure data persistence before responding to clients
//!
//! The tracking is based on monotonically increasing [`IOId`] values that identify each I/O
//! operation. When storage completes an operation, it notifies RaftCore, which updates the
//! progress channels.

use std::fmt;

use crate::LogId;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::Vote;
use crate::async_runtime::watch::RecvError;
use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::watch::WatchSender;
use crate::display_ext::display_option::DisplayOptionExt;
use crate::raft_state::IOId;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;

/// Handle for tracking log I/O flush progress.
///
/// Returns `None` if no I/O has completed yet (e.g., on a newly started node before any writes).
/// Returns `Some(FlushPoint)` containing the vote and last log ID after the first flush
/// completes.
pub type LogProgress<C> = WatchProgress<C, Option<FlushPoint<C>>>;

/// Handle for tracking vote I/O flush progress.
///
/// Returns `None` if no vote has been flushed yet.
/// Returns `Some(Vote)` containing the last flushed vote.
pub type VoteProgress<C> = WatchProgress<C, Option<Vote<C>>>;

/// State of the most recently flushed log I/O operation.
///
/// This represents the durable state of the log storage after a flush completes. It includes:
/// - The vote(leader) under which the I/O was submitted
/// - The last log entry that was written (if any logs were appended)
///
/// # Ordering
///
/// Ordered lexicographically as `(vote, last_log_id)`:
/// - Higher term always > lower term
/// - Same term: higher node_id > lower node_id (for non-committed votes)
/// - Same vote: longer log (higher index) > shorter log
///
/// This enables `wait_until_ge()` to wait for specific progress milestones.
///
/// # Special Cases
///
/// - `!vote.is_committed() && last_log_id.is_none()`: A candidate's vote is accepted but it has not
///   yet become leader (no AppendEntries received).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd)]
pub struct FlushPoint<C>
where C: RaftTypeConfig
{
    /// The vote(leader) under which this I/O operation was submitted.
    pub vote: Vote<C>,

    /// The last log entry that was flushed, or `None` if only a vote was saved without appending
    /// logs.
    pub last_log_id: Option<LogId<C>>,
}

impl<C> fmt::Display for FlushPoint<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FlushPoint({}, {})", self.vote, self.last_log_id.display(),)
    }
}

impl<C> FlushPoint<C>
where C: RaftTypeConfig
{
    pub fn new(vote: Vote<C>, last_log_id: Option<LogId<C>>) -> Self {
        Self { vote, last_log_id }
    }
}

/// Watch handle for tracking I/O flush progress.
///
/// Provides two operations:
/// - [`get()`](Self::get): Get current progress state immediately
/// - [`wait_until_ge()`](Self::wait_until_ge): Wait asynchronously until progress reaches a
///   threshold
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

    /// Get the current flushed I/O progress state immediately without waiting.
    ///
    /// This returns a snapshot of the most recent flushed I/O operation. The value may become
    /// stale immediately after reading as new I/O operations complete concurrently.
    pub fn get(&self) -> T {
        self.inner.borrow_watched().clone()
    }
}

/// I/O flush progress watch handles.
///
/// This struct maintains the receiving ends of watch channels that track I/O flush progress.
/// It provides two independent progress streams:
///
/// - **Log progress**: Tracks all I/O operations (both vote saves and log appends). Updated
///   whenever any data is flushed to storage.
///
/// - **Vote progress**: Tracks only vote changes. Updated when the vote value changes (new term or
///   leader), not on every log append.
///
/// The separation allows applications to efficiently wait for different kinds of progress:
/// - Use log progress to wait for specific log entries to be durable
/// - Use vote progress to wait for leadership changes to be persisted
pub(crate) struct IOProgressWatcher<C>
where C: RaftTypeConfig
{
    /// Receiver for log I/O progress (vote + log appends).
    log: WatchReceiverOf<C, Option<FlushPoint<C>>>,

    /// Receiver for vote I/O progress (vote saves only, or implied by log appends).
    vote: WatchReceiverOf<C, Option<Vote<C>>>,
}

impl<C> IOProgressWatcher<C>
where C: RaftTypeConfig
{
    /// Create a new progress watcher/sender pair.
    ///
    /// Returns `(IOProgressSender, IOProgressWatcher)` where:
    /// - The sender is used by RaftCore to publish flush notifications
    /// - The watcher is used to create watch handles for applications
    pub(crate) fn new() -> (IOProgressSender<C>, Self) {
        let (log_tx, log) = C::watch_channel(None);
        let (vote_tx, vote) = C::watch_channel(None);

        let sender = IOProgressSender { log_tx, vote_tx };
        let watcher = Self { log, vote };

        (sender, watcher)
    }

    /// Create a watch handle for log I/O flush progress.
    ///
    /// Each call creates a new independent handle. Multiple handles can watch the same progress
    /// concurrently.
    pub(crate) fn log_progress(&self) -> LogProgress<C> {
        LogProgress::new(self.log.clone())
    }

    /// Create a watch handle for vote I/O flush progress.
    ///
    /// Each call creates a new independent handle. Multiple handles can watch the same progress
    /// concurrently.
    pub(crate) fn vote_progress(&self) -> VoteProgress<C> {
        VoteProgress::new(self.vote.clone())
    }
}

/// Sender for publishing I/O flush progress notifications.
///
/// Used internally by RaftCore to notify watchers when I/O operations complete.
/// The sender maintains two independent channels (log and vote) to allow efficient
/// filtering of notifications.
pub(crate) struct IOProgressSender<C>
where C: RaftTypeConfig
{
    /// Sender for log I/O progress (includes all I/O operations).
    pub(crate) log_tx: WatchSenderOf<C, Option<FlushPoint<C>>>,

    /// Sender for vote I/O progress (vote-specific updates).
    ///
    /// Note: Uses `Vote<C>` (the internal concrete type) instead of `VoteOf<C>` (the trait)
    /// because `PartialOrd` is required for progress tracking but user-provided `VoteOf<C>`
    /// implementations may not provide it.
    pub(crate) vote_tx: WatchSenderOf<C, Option<Vote<C>>>,
}

impl<C> IOProgressSender<C>
where C: RaftTypeConfig
{
    /// Publish an I/O flush completion notification to watchers.
    ///
    /// Updates progress channels conditionally:
    /// - **vote_tx**: Updated only when the vote changes (new term or leader)
    /// - **log_tx**: Updated when either vote or log_id changes (any I/O progress)
    ///
    /// This separation allows applications to efficiently wait for leadership changes
    /// without being notified of every log append.
    ///
    /// # Arguments
    ///
    /// * `io_id` - The I/O operation that just completed. `None` means no progress to report.
    pub(crate) fn send_log_progress(&self, io_id: Option<IOId<C>>) {
        self.do_send_log_progress(io_id);
    }

    /// Internal implementation of `send_log_progress` that returns an option, just for easy return.
    fn do_send_log_progress(&self, io_id: Option<IOId<C>>) -> Option<()> {
        tracing::debug!("send_log_progress: try to update to :{}", io_id.display());

        let (vote, log_id) = io_id?.to_vote_and_log_id();

        {
            let vote = vote.clone();

            self.vote_tx.send_if_modified(move |prev| {
                if prev.as_ref() != Some(&vote) {
                    tracing::debug!("send_log_progress: udpate vote to :{}", vote);
                    *prev = Some(vote);
                    true
                } else {
                    false
                }
            });
        }

        self.log_tx.send_if_modified(move |prev| {
            let x = Some(FlushPoint::new(vote, log_id));
            if prev.as_ref() != x.as_ref() {
                tracing::debug!("send_log_progress: udpate log to :{}", x.display());
                *prev = x;
                true
            } else {
                false
            }
        });

        Some(())
    }
}
