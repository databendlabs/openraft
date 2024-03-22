use std::fmt;

/// The request id of a replication action.
///
/// HeartBeat has not payload and does not need a request id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RequestId {
    HeartBeat,
    AppendEntries { id: u64 },
    Snapshot { id: u64 },
}

impl RequestId {
    pub(crate) fn new_heartbeat() -> Self {
        Self::HeartBeat
    }

    pub(crate) fn new_append_entries(id: u64) -> Self {
        Self::AppendEntries { id }
    }

    pub(crate) fn new_snapshot(id: u64) -> Self {
        Self::Snapshot { id }
    }

    pub(crate) fn request_id(&self) -> Option<u64> {
        match self {
            Self::HeartBeat => None,
            Self::AppendEntries { id } => Some(*id),
            Self::Snapshot { id } => Some(*id),
        }
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeartBeat => write!(f, "HeartBeat"),
            Self::AppendEntries { id } => write!(f, "AppendEntries({})", id),
            Self::Snapshot { id } => write!(f, "Snapshot({})", id),
        }
    }
}
