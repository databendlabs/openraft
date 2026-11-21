/// The current replication task or snapshot transfer must stop.
///
/// This can follow an explicit cancellation or happen without one, for example
/// when the state machine worker can no longer answer a snapshot request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[error("Replication is closed: {reason}")]
pub struct ReplicationClosed {
    reason: String,
}

impl ReplicationClosed {
    /// Create a new ReplicationClosed error with the given reason.
    pub fn new(reason: impl ToString) -> Self {
        Self {
            reason: reason.to_string(),
        }
    }
}
