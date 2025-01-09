use std::cmp::Ordering;

use crate::alias::CommittedLeaderIdOf;
use crate::log_id::ord_log_id::OrdLogId;
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

    fn cmp(&self, other: &Self) -> Ordering {
        self.ord_by().cmp(&other.ord_by())
    }

    fn to_ordered(&self) -> Option<OrdLogId<C>> {
        self.clone().into_ordered()
    }

    fn into_ordered(self) -> Option<OrdLogId<C>>
    where Self: Sized;
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
        self.as_ref().map(|x| x.leader_id())
    }

    fn ord_by(&self) -> Option<RefLogId<'_, C>> {
        self.as_ref().map(|x| x.ord_by())
    }

    fn into_ordered(self) -> Option<OrdLogId<C>>
    where Self: Sized {
        self.map(|x| x.ordered())
    }
}
