/// Enum representing the name of each `Notification` variant.
///
/// This provides an efficient way to identify notification types without
/// string comparisons, useful for logging, metrics, and debugging.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NotificationName {
    VoteResponse,
    HigherVote,
    StorageError,
    LocalIO,
    ReplicationProgress,
    HeartbeatProgress,
    StateMachine,
    Tick,
}

impl NotificationName {
    /// Total number of variants.
    #[allow(dead_code)]
    pub const COUNT: usize = 8;

    /// All variants in canonical order.
    #[allow(dead_code)]
    pub const ALL: &'static [NotificationName] = &[
        NotificationName::VoteResponse,
        NotificationName::HigherVote,
        NotificationName::StorageError,
        NotificationName::LocalIO,
        NotificationName::ReplicationProgress,
        NotificationName::HeartbeatProgress,
        NotificationName::StateMachine,
        NotificationName::Tick,
    ];

    /// Returns the index of this variant for array-based storage.
    #[allow(dead_code)]
    pub const fn index(&self) -> usize {
        match self {
            NotificationName::VoteResponse => 0,
            NotificationName::HigherVote => 1,
            NotificationName::StorageError => 2,
            NotificationName::LocalIO => 3,
            NotificationName::ReplicationProgress => 4,
            NotificationName::HeartbeatProgress => 5,
            NotificationName::StateMachine => 6,
            NotificationName::Tick => 7,
        }
    }

    #[allow(dead_code)]
    pub const fn as_str(&self) -> &'static str {
        match self {
            NotificationName::VoteResponse => "Notify::VoteResponse",
            NotificationName::HigherVote => "Notify::HigherVote",
            NotificationName::StorageError => "Notify::StorageError",
            NotificationName::LocalIO => "Notify::LocalIO",
            NotificationName::ReplicationProgress => "Notify::ReplicationProgress",
            NotificationName::HeartbeatProgress => "Notify::HeartbeatProgress",
            NotificationName::StateMachine => "Notify::StateMachine",
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
