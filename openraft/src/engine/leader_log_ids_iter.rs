use crate::LogId;
use crate::vote::RaftCommittedLeaderId;

/// Iterator over log IDs in a [`LeaderLogIds`](super::leader_log_ids::LeaderLogIds) range.
///
/// Uses half-open range `[start, end)` where `start` is inclusive and `end` is exclusive.
/// When `start == end`, the iterator is exhausted.
/// Implements `DoubleEndedIterator` and `ExactSizeIterator`.
#[derive(Debug, Clone)]
pub(crate) struct LeaderLogIdsIter<CLID>
where CLID: RaftCommittedLeaderId
{
    committed_leader_id: CLID,

    /// Next index to yield from the front (inclusive).
    start: u64,

    /// One past the last index to yield (exclusive).
    end: u64,
}

impl<CLID> LeaderLogIdsIter<CLID>
where CLID: RaftCommittedLeaderId
{
    /// Create an iterator over the log IDs in `[start, end)`, which is empty when `start >= end`.
    ///
    /// # Panics
    ///
    /// Panics if the range holds more log IDs than `usize` can count, which happens only on a
    /// target whose `usize` is narrower than `u64`: the iterator reports its exact length as a
    /// `usize`. The range is checked by its length, not by its ends, so a short range at a large
    /// index is accepted.
    pub(crate) fn new(committed_leader_id: CLID, start: u64, end: u64) -> Self {
        let span = end.saturating_sub(start);
        if usize::try_from(span).is_err() {
            panic!("the log id range [{start}, {end}) holds {span} log ids, more than usize can count");
        }

        Self {
            committed_leader_id,
            start,
            end,
        }
    }

    /// The number of log IDs left; [`Self::new`] checked that it fits `usize`, and iterating only
    /// shrinks it.
    fn len(&self) -> usize {
        if self.start >= self.end {
            0
        } else {
            (self.end - self.start) as usize
        }
    }
}

impl<CLID> Iterator for LeaderLogIdsIter<CLID>
where CLID: RaftCommittedLeaderId
{
    type Item = LogId<CLID>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start >= self.end {
            return None;
        }

        let index = self.start;
        self.start += 1;
        Some(LogId::<CLID>::new(self.committed_leader_id.clone(), index))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<CLID> DoubleEndedIterator for LeaderLogIdsIter<CLID>
where CLID: RaftCommittedLeaderId
{
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.start >= self.end {
            return None;
        }

        self.end -= 1;
        Some(LogId::<CLID>::new(self.committed_leader_id.clone(), self.end))
    }
}

impl<CLID> ExactSizeIterator for LeaderLogIdsIter<CLID> where CLID: RaftCommittedLeaderId {}

#[cfg(test)]
mod leader_log_ids_iter_test;
