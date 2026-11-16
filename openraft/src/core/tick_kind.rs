use std::fmt;

/// Which ticker instance sent a [`Notification::Tick`](crate::core::notification::Notification).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TickKind {
    /// The periodic ticker: it drives heartbeats and every election check except a follower's.
    Heartbeat,

    /// The election ticker: it ticks when a follower's election may be due.
    Election,
}

impl fmt::Display for TickKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Heartbeat => write!(f, "heartbeat"),
            Self::Election => write!(f, "election"),
        }
    }
}
