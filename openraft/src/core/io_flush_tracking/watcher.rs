use super::FlushPoint;
use crate::RaftTypeConfig;
use crate::Vote;
use crate::core::io_flush_tracking::LogProgress;
use crate::core::io_flush_tracking::VoteProgress;
use crate::core::io_flush_tracking::sender::IoProgressSender;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::WatchReceiverOf;

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
