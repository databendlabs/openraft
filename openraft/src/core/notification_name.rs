use openraft_macros::VariantName;
use openraft_macros::since;

/// Enum representing the name of each `Notification` variant.
///
/// This provides an efficient way to identify notification types without
/// string comparisons, useful for logging, metrics, and debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(VariantName)]
#[variant_name(prefix = "Notify::")]
#[since(
    version = "0.10.0",
    change = "renamed from `CheckPendingLinearizableReads` to `PendingReadDeadlineReached`"
)]
pub enum NotificationName {
    VoteResponse,
    PreVoteResponse,
    HigherVote,
    StorageError,
    LocalIO,
    ReplicationProgress,
    HeartbeatProgress,
    StateMachine,
    Tick,
    PendingReadDeadlineReached,
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
