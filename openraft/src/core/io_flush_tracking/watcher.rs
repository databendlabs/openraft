use crate::RaftTypeConfig;
use crate::Vote;
use crate::async_runtime::watch::WatchSender;
use crate::core::io_flush_tracking::FlushPoint;
use crate::core::io_flush_tracking::LogProgress;
use crate::core::io_flush_tracking::VoteProgress;
use crate::display_ext::display_option::DisplayOptionExt;
use crate::raft_state::IOId;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;

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
pub(crate) struct IoProgressWatcher<C>
where C: RaftTypeConfig
{
    /// Receiver for log I/O progress (vote + log appends).
    log: WatchReceiverOf<C, Option<FlushPoint<C>>>,

    /// Receiver for vote I/O progress (vote saves only, or implied by log appends).
    vote: WatchReceiverOf<C, Option<Vote<C>>>,
}

impl<C> IoProgressWatcher<C>
where C: RaftTypeConfig
{
    /// Create a new progress watcher/sender pair.
    ///
    /// Returns `(IoProgressSender, IoProgressWatcher)` where:
    /// - The sender is used by RaftCore to publish flush notifications
    /// - The watcher is used to create watch handles for applications
    pub(crate) fn new() -> (IoProgressSender<C>, Self) {
        let (log_tx, log) = C::watch_channel(None);
        let (vote_tx, vote) = C::watch_channel(None);

        let sender = IoProgressSender { log_tx, vote_tx };
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
pub(crate) struct IoProgressSender<C>
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

impl<C> IoProgressSender<C>
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
