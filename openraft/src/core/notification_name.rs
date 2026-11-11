use openraft_macros::VariantName;
use openraft_macros::since;

/// Enum representing the name of each `Notification` variant.
///
/// This provides an efficient way to identify notification types without
/// string comparisons, useful for logging, metrics, and debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(VariantName)]
#[variant_name(prefix = "Notify::")]
#[since(version = "0.10.1", change = "marked #[non_exhaustive]")]
#[since(version = "0.10.0", change = "Added `LogPurged`")]
#[since(
    version = "0.10.0",
    change = "renamed from `CheckPendingLinearizableReads` to `PendingReadDeadlineReached`"
)]
#[non_exhaustive]
pub enum NotificationName {
    /// A response to a vote request.
    VoteResponse,
    /// A response to a Pre-Vote request.
    PreVoteResponse,
    /// A leader saw a higher vote while replicating.
    HigherVote,
    /// A local storage error that stops `RaftCore`.
    StorageError,
    /// A local storage IO operation completed.
    LocalIO,
    /// A replication worker reported progress.
    ReplicationProgress,
    /// A heartbeat worker reported progress.
    HeartbeatProgress,
    /// A state machine worker returned the result of a command.
    StateMachine,
    /// A tick that wakes `RaftCore` to check timeouts.
    Tick,
    /// The earliest pending linearizable read reached its deadline.
    PendingReadDeadlineReached,
    /// A log purge completed in storage.
    LogPurged,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_name_index() {
        assert_eq!(NotificationName::COUNT, NotificationName::ALL.len());

        for (i, name) in NotificationName::ALL.iter().enumerate() {
            assert_eq!(
                name.index(),
                i,
                "NotificationName::{:?} index mismatch: expected {}, got {}",
                name,
                i,
                name.index()
            );
        }
    }
}
