use openraft_macros::since;

use crate::log_id::raft_log_id::RaftLogId;

/// This helper trait extracts information from an `Option<LogId>`.
#[since(version = "0.10.0", change = "removed `C: RaftTypeConfig` generic parameter")]
pub trait LogIdOptionExt {
    /// Returns the log index if it is not a `None`.
    fn index(&self) -> Option<u64>;

    /// Returns the next log index.
    ///
    /// If self is `None`, it returns 0.
    fn next_index(&self) -> u64;
}

impl<T> LogIdOptionExt for Option<T>
where T: RaftLogId
{
    fn index(&self) -> Option<u64> {
        self.as_ref().map(|x| x.index())
    }

    fn next_index(&self) -> u64 {
        match self {
            None => 0,
            Some(log_id) => log_id.index() + 1,
        }
    }
}
