use std::fmt;

use crate::LogId;
use crate::engine::leader_log_ids_iter::LeaderLogIdsIter;
use crate::log_id::ref_log_id::RefLogId;
use crate::vote::RaftCommittedLeaderId;

/// A non-empty range of log IDs belonging to a Leader.
///
/// This struct represents a contiguous range of log IDs that share the same
/// committed leader ID. It provides efficient access to the first and last
/// log IDs, and can be iterated over to produce all log IDs in the range.
///
/// The range is `[first, last]` (both inclusive).
/// This struct always represents a non-empty range; use `Option<LeaderLogIds>`
/// when an empty range is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeaderLogIds<CLID>
where CLID: RaftCommittedLeaderId
{
    committed_leader_id: CLID,

    /// First index (inclusive).
    first: u64,

    /// Last index (inclusive).
    last: u64,
}

impl<CLID> fmt::Display for LeaderLogIds<CLID>
where CLID: RaftCommittedLeaderId
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:[{}, {}]", self.committed_leader_id, self.first, self.last)
    }
}

impl<CLID> LeaderLogIds<CLID>
where CLID: RaftCommittedLeaderId
{
    /// Create a new log ID range `[first, last]`.
    ///
    /// The caller must pass `first <= last`. Only a build with debug assertions checks it, and
    /// panics when it does not hold.
    pub(crate) fn new(committed_leader_id: CLID, first: u64, last: u64) -> Self {
        debug_assert!(first <= last, "first {} must be <= last {}", first, last);
        Self {
            committed_leader_id,
            first,
            last,
        }
    }

    /// Returns a reference to the last log ID in the range.
    #[cfg_attr(not(feature = "runtime-stats"), allow(dead_code))]
    pub(crate) fn last_ref(&self) -> RefLogId<'_, CLID> {
        RefLogId::new(&self.committed_leader_id, self.last)
    }

    /// Returns the log ID at the specified index.
    ///
    /// The caller must pass an `index` in `[first, last]`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range, but only in a build with debug assertions. Other builds
    /// do not check it, and return a log ID at `index` even outside the range.
    #[allow(dead_code)]
    pub(crate) fn get(&self, index: u64) -> LogId<CLID> {
        debug_assert!(
            index >= self.first && index <= self.last,
            "index {} out of range [{}, {}]",
            index,
            self.first,
            self.last
        );
        LogId::<CLID>::new(self.committed_leader_id.clone(), index)
    }

    /// Returns a reference to the log ID at the specified index.
    ///
    /// The caller must pass an `index` in `[first, last]`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range, but only in a build with debug assertions. Other builds
    /// do not check it, and return a log ID at `index` even outside the range.
    #[allow(dead_code)]
    pub(crate) fn ref_at(&self, index: u64) -> RefLogId<'_, CLID> {
        debug_assert!(
            index >= self.first && index <= self.last,
            "index {} out of range [{}, {}]",
            index,
            self.first,
            self.last
        );
        RefLogId::new(&self.committed_leader_id, index)
    }

    /// Returns the number of log IDs in the range.
    ///
    /// # Panics
    ///
    /// Panics if the count does not fit in `usize`: for `[0, u64::MAX]`, or on a 32-bit target for
    /// a range of more than `u32::MAX` log IDs.
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        if self.first > self.last {
            0
        } else {
            let len = (self.last - self.first).checked_add(1).and_then(|n| usize::try_from(n).ok());
            len.unwrap_or_else(|| panic!("the number of log ids in {} does not fit in usize", self))
        }
    }
}

impl<CLID> IntoIterator for LeaderLogIds<CLID>
where CLID: RaftCommittedLeaderId
{
    type Item = LogId<CLID>;
    type IntoIter = LeaderLogIdsIter<CLID>;

    /// # Panics
    ///
    /// Panics if the last index is `u64::MAX`, which has no successor to end the half-open
    /// iterator range `[first, last + 1)`.
    fn into_iter(self) -> Self::IntoIter {
        let end = self.last.checked_add(1);
        let end = end.unwrap_or_else(|| panic!("{} can not be iterated: its last index has no successor", self));
        LeaderLogIdsIter::new(self.committed_leader_id, self.first, end)
    }
}

#[cfg(test)]
mod leader_log_ids_test;
