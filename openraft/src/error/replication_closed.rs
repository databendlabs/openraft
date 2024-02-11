/// Replication is closed intentionally.
///
/// No further replication action should be taken.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[error("Replication is closed: {reason}")]
pub struct ReplicationClosed {
    reason: String,
}

impl ReplicationClosed {
    pub fn new(reason: impl ToString) -> Self {
        Self {
            reason: reason.to_string(),
        }
    }
}
