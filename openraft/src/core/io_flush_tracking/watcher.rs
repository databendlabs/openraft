use super::FlushPoint;
use crate::RaftTypeConfig;
use crate::Vote;
use crate::core::io_flush_tracking::AppliedProgress;
use crate::core::io_flush_tracking::CommitProgress;
use crate::core::io_flush_tracking::LogProgress;
use crate::core::io_flush_tracking::SnapshotProgress;
use crate::core::io_flush_tracking::VoteProgress;
use crate::core::io_flush_tracking::sender::IoProgressSender;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::WatchReceiverOf;

/// I/O flush progress watch handles.
///
/// This struct maintains the receiving ends of watch channels that track I/O flush progress.
/// It provides five independent progress streams:
///
/// - **Log progress**: Tracks all I/O operations (both vote saves and log appends). Updated
///   whenever any data is flushed to storage.
///
/// - **Vote progress**: Tracks only vote changes. Updated when the vote value changes (new term or
///   leader), not on every log append.
///
/// - **Commit progress**: Tracks when committed logs are submitted to the state machine.
///
/// - **Snapshot progress**: Tracks snapshot persistence. Updated when snapshots are built or
///   installed and persisted.
///
/// - **Applied progress**: Tracks logs applied to the state machine. Updated when the last applied
///   log id advances.
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

    /// Receiver for commit log progress (state machine submission).
    commit: WatchReceiverOf<C, Option<LogIdOf<C>>>,

    /// Receiver for snapshot persistence progress.
    snapshot: WatchReceiverOf<C, Option<LogIdOf<C>>>,

    /// Receiver for applied log progress (state machine application).
    apply: WatchReceiverOf<C, Option<LogIdOf<C>>>,
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
        let (commit_tx, commit) = C::watch_channel(None);
        let (snapshot_tx, snapshot) = C::watch_channel(None);
        let (apply_tx, apply) = C::watch_channel(None);

        let sender = IoProgressSender {
            log_tx,
            vote_tx,
            commit_tx,
            snapshot_tx,
            apply_tx,
        };
        let watcher = Self {
            log,
            vote,
            commit,
            snapshot,
            apply,
        };

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

    /// Create a watch handle for commit log progress.
    ///
    /// Each call creates a new independent handle. Multiple handles can watch the same progress
    /// concurrently.
    pub(crate) fn commit_progress(&self) -> CommitProgress<C> {
        CommitProgress::new(self.commit.clone())
    }

    /// Create a watch handle for snapshot persistence progress.
    ///
    /// Each call creates a new independent handle. Multiple handles can watch the same progress
    /// concurrently.
    pub(crate) fn snapshot_progress(&self) -> SnapshotProgress<C> {
        SnapshotProgress::new(self.snapshot.clone())
    }

    /// Create a watch handle for applied log progress.
    ///
    /// Each call creates a new independent handle. Multiple handles can watch the same progress
    /// concurrently.
    pub(crate) fn apply_progress(&self) -> AppliedProgress<C> {
        AppliedProgress::new(self.apply.clone())
    }
}
