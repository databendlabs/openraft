use std::cmp::Ordering;

use crate::alias::CommittedLeaderIdOf;
use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::log_id::ref_log_id::RefLogId;
use crate::RaftLogId;
use crate::RaftTypeConfig;

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

    fn leader_id(&self) -> Option<&CommittedLeaderIdOf<C>>;

    /// Returns the ordering key of the log id.
    fn ord_by(&self) -> Option<RefLogId<'_, C>>;

    fn ref_log_id(&self) -> Option<RefLogId<'_, C>> {
        self.ord_by()
    }

    fn cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(&self.ord_by(), &other.ord_by())
    }
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

    fn leader_id(&self) -> Option<&CommittedLeaderIdOf<C>> {
        self.as_ref().map(|x| x.committed_leader_id())
    }

    fn ord_by(&self) -> Option<RefLogId<'_, C>> {
        self.as_ref().map(|x| x.ord_by())
    }
}
