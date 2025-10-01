use crate::RaftTypeConfig;
use crate::log_id::raft_log_id::RaftLogId;

/// This helper trait extracts information from an `Option<LogId>`.
pub trait LogIdOptionExt<C>
where C: RaftTypeConfig
{
    /// Returns the log index if it is not a `None`.
    fn index(&self) -> Option<u64>;

    /// Returns the next log index.
    ///
    /// If self is `None`, it returns 0.
    fn next_index(&self) -> u64;
}

impl<C, T> LogIdOptionExt<C> for Option<T>
where
    C: RaftTypeConfig,
    T: RaftLogId<C>,
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
