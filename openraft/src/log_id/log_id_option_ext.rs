use crate::LogId;
use crate::NodeId;

/// This helper trait extracts information from an `Option<LogId>`.
pub trait LogIdOptionExt {
    /// Returns the log index if it is not a `None`.
    fn index(&self) -> Option<u64>;

    /// Returns the next log index.
    ///
    /// If self is `None`, it returns 0.
    fn next_index(&self) -> u64;
}

impl<NID: NodeId> LogIdOptionExt for Option<LogId<NID>> {
    fn index(&self) -> Option<u64> {
        self.map(|x| x.index)
    }

    fn next_index(&self) -> u64 {
        match self {
            None => 0,
            Some(log_id) => log_id.index + 1,
        }
    }
}

impl<NID: NodeId> LogIdOptionExt for Option<&LogId<NID>> {
    fn index(&self) -> Option<u64> {
        self.map(|x| x.index)
    }

    fn next_index(&self) -> u64 {
        match self {
            None => 0,
            Some(log_id) => log_id.index + 1,
        }
    }
}
