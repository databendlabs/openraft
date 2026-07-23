/// Enum representing the name of each `Notification` variant.
///
/// This provides an efficient way to identify notification types without
/// string comparisons, useful for logging, metrics, and debugging.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NotificationName {
    VoteResponse,
    PreVoteResponse,
    HigherVote,
    StorageError,
    LocalIO,
    ReplicationProgress,
    HeartbeatProgress,
    StateMachine,
    SnapshotTransmitted,
    Tick,
}

impl NotificationName {
    /// Total number of variants.
    #[allow(dead_code)]
    pub const COUNT: usize = 10;

    /// All variants in canonical order.
    #[allow(dead_code)]
    pub const ALL: &'static [NotificationName] = &[
        NotificationName::VoteResponse,
        NotificationName::PreVoteResponse,
        NotificationName::HigherVote,
        NotificationName::StorageError,
        NotificationName::LocalIO,
        NotificationName::ReplicationProgress,
        NotificationName::HeartbeatProgress,
        NotificationName::StateMachine,
        NotificationName::SnapshotTransmitted,
        NotificationName::Tick,
    ];

    /// Returns the index of this variant for array-based storage.
    #[allow(dead_code)]
    pub const fn index(&self) -> usize {
        match self {
            NotificationName::VoteResponse => 0,
            NotificationName::PreVoteResponse => 1,
            NotificationName::HigherVote => 2,
            NotificationName::StorageError => 3,
            NotificationName::LocalIO => 4,
            NotificationName::ReplicationProgress => 5,
            NotificationName::HeartbeatProgress => 6,
            NotificationName::StateMachine => 7,
            NotificationName::SnapshotTransmitted => 8,
            NotificationName::Tick => 9,
        }
    }

    #[allow(dead_code)]
    pub const fn as_str(&self) -> &'static str {
        match self {
            NotificationName::VoteResponse => "Notify::VoteResponse",
            NotificationName::PreVoteResponse => "Notify::PreVoteResponse",
            NotificationName::HigherVote => "Notify::HigherVote",
            NotificationName::StorageError => "Notify::StorageError",
            NotificationName::LocalIO => "Notify::LocalIO",
            NotificationName::ReplicationProgress => "Notify::ReplicationProgress",
            NotificationName::HeartbeatProgress => "Notify::HeartbeatProgress",
            NotificationName::StateMachine => "Notify::StateMachine",
            NotificationName::SnapshotTransmitted => "Notify::SnapshotTransmitted",
            NotificationName::Tick => "Notify::Tick",
        }
    }
}

impl std::fmt::Display for NotificationName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
