use std::ops::RangeInclusive;

use crate::RaftLogReader;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::engine::leader_log_ids::LeaderLogIds;
use crate::log_id::option_raft_log_id_ext::OptionRaftLogIdExt;
use crate::log_id::raft_log_id::RaftLogId;
use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::log_id::ref_log_id::RefLogId;
use crate::storage::RaftLogReaderExt;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;

/// Efficient storage for log ids.
///
/// It stores the last purged log id separately, and the last log id of each leader.
/// Each leader has exactly one entry in `key_log_ids` (their last log id).
///
/// For example, given logs from Leader1: 1-3, Leader2: 4-6, Leader3: 7-8, purged at 0:
/// - `purged` = Some(log_id(0))
/// - `key_log_ids` = [log_id(1,3), log_id(2,6), log_id(3,8)]
#[derive(Default, Debug, Clone)]
#[derive(PartialEq, Eq)]
pub struct LogIdList<C>
where C: RaftTypeConfig
{
    /// The last purged log id, if any logs have been purged.
    purged: Option<LogIdOf<C>>,

    /// The last log id of each leader. Each leader has exactly one entry.
    key_log_ids: Vec<LogIdOf<C>>,
}

impl<C> LogIdList<C>
where C: RaftTypeConfig
{
    /// Helper function to push a log entry, replacing last entry if same leader.
    ///
    /// When the same leader appears multiple times due to range subdivision in the binary
    /// search, we keep only the entry with the greatest index (the true last of that leader).
    ///
    /// The binary search guarantees logs are processed in non-decreasing index order,
    /// so we can simply replace when we see the same leader again.
    fn push_key_log_id(res: &mut Vec<LogIdOf<C>>, log_id: LogIdOf<C>) {
        if let Some(last_ent) = res.last_mut()
            && last_ent.committed_leader_id() == log_id.committed_leader_id()
        {
            // Same leader: replace with the newer entry (which has greater or equal index)
            debug_assert!(
                log_id.index() >= last_ent.index(),
                "Binary search invariant violated: expected log_id.index({}) >= last_ent.index({})",
                log_id.index(),
                last_ent.index()
            );
            *last_ent = log_id;
            return;
        }

        // Different leader or empty result: push new entry
        res.push(log_id);
    }

    /// Load the last log id of each leader from storage.
    ///
    /// Each leader has exactly one entry in the result (their last log id).
    /// Uses a binary search algorithm to find leadership change boundaries.
    ///
    /// Given logs `[(2,2),(2,3),(5,4),(5,5)]`, this returns `[(2,3),(5,5)]`.
    ///
    /// Algorithm: Binary search to find boundaries between leaders and their last logs.
    /// ```text
    /// input:
    /// A---------------C
    ///
    /// load the mid log-id, then compare the first, the middle, and the last:
    ///
    /// A---------------A : push_res(A_last);                   // single leader, push last
    /// A-------A-------C : find(A_mid, C)                      // boundary in right half (AAC)
    /// A-------B-------C : find(A,B); find(B,C)                // boundaries in both halves (ABC)
    /// A-------C-------C : find(A, C_mid); find(C_mid, C_last) // boundary in left, find C's last in right (ACC)
    /// ```
    ///
    /// The key insight is that when mid and last have the same leader (Case ACC), we must:
    /// 1. Search the left half `(first, mid)` to find where the previous leader ends
    /// 2. Search the right half `(mid, last)` to find where leader C actually ends
    ///
    /// This ensures we find the true last log of each leader, not just intermediate positions.
    pub(crate) async fn get_key_log_ids<LR>(
        range: RangeInclusive<LogIdOf<C>>,
        sto: &mut LR,
    ) -> Result<Vec<LogIdOf<C>>, StorageError<C>>
    where
        LR: RaftLogReader<C> + ?Sized,
    {
        let first = range.start().clone();
        let last = range.end().clone();

        let mut res: Vec<LogIdOf<C>> = vec![];

        // Recursion stack
        let mut stack = vec![(first, last)];

        loop {
            let Some((first, last)) = stack.pop() else {
                break;
            };

            // Case AA: Same leader for entire range - push the last
            if first.committed_leader_id() == last.committed_leader_id() {
                Self::push_key_log_id(&mut res, last);
                continue;
            }

            // Two adjacent logs with different leader_id:
            // first is the last of leader A, last is the last of leader B (for this span)
            if first.index() + 1 == last.index() {
                Self::push_key_log_id(&mut res, first);
                Self::push_key_log_id(&mut res, last);
                continue;
            }

            // A*C cases:

            let mid_index = (first.index() + last.index()) / 2;
            let mid = sto.get_log_id(mid_index).await?;

            // Case AAC: boundary is between mid and last
            if first.committed_leader_id() == mid.committed_leader_id() {
                stack.push((mid, last));
                continue;
            }

            // Case ACC or ABC: search both halves.
            // - ACC: boundary in left half; right half finds where leader C actually ends
            // - ABC: boundaries in both halves
            stack.push((mid.clone(), last));
            stack.push((first, mid));
        }

        Ok(res)
    }
}

impl<C> LogIdList<C>
where C: RaftTypeConfig
{
    /// Create a new `LogIdList`.
    ///
    /// It stores the last purged log id and the last log id of each leader.
    ///
    /// - `purged`: The last purged log id, if any logs have been purged.
    /// - `key_log_ids`: The last log id of each leader. Each leader has exactly one entry.
    pub fn new(purged: Option<LogIdOf<C>>, key_log_ids: impl IntoIterator<Item = LogIdOf<C>>) -> Self {
        Self {
            purged,
            key_log_ids: key_log_ids.into_iter().collect(),
        }
    }

    /// Get the last purged log id, if any.
    pub(crate) fn purged(&self) -> Option<&LogIdOf<C>> {
        self.purged.as_ref()
    }

    /// Get the first log index (purged.index + 1, or 0 if nothing purged).
    pub(crate) fn first_index(&self) -> u64 {
        self.purged.next_index()
    }

    /// Extends a list of `log_id` that are proposed by the same leader.
    ///
    /// The log ids in the input have to be continuous.
    pub(crate) fn extend_from_same_leader<LID, I>(&mut self, new_ids: I)
    where
        LID: RaftLogId<C>,
        I: IntoIterator<Item = LID>,
        <I as IntoIterator>::IntoIter: DoubleEndedIterator,
    {
        let mut it = new_ids.into_iter();
        if let Some(first) = it.next() {
            self.append(first.to_log_id());

            if let Some(last) = it.next_back() {
                debug_assert_eq!(last.committed_leader_id(), first.committed_leader_id());

                if last != first {
                    self.append(last.to_log_id());
                }
            }
        }
    }

    /// Extends with a list of `log_id`.
    pub(crate) fn extend<LID, I>(&mut self, new_ids: I)
    where
        LID: RaftLogId<C>,
        I: IntoIterator<Item = LID>,
    {
        for log_id in new_ids {
            self.append(log_id.to_log_id());
        }
    }

    /// Append a new `log_id`.
    ///
    /// With last-per-leader storage:
    /// - If same leader as the last entry, replace the last entry
    /// - If different leader, push a new entry
    pub(crate) fn append(&mut self, new_log_id: LogIdOf<C>) {
        #[cfg(debug_assertions)]
        if let Some(last) = self.last() {
            debug_assert!(new_log_id > *last, "new_log_id: {}, last: {}", new_log_id, last);
        }
        Self::push_key_log_id(&mut self.key_log_ids, new_log_id);
    }

    /// Delete log ids from `at`, inclusive.
    ///
    /// With last-per-leader storage:
    /// - Entry at position i covers a range starting after entry[i-1] (or purged+1 for i=0)
    /// - If truncation point is within an entry's range, update that entry to (at-1)
    // leader_id: Copy is feature gated
    #[allow(clippy::clone_on_copy)]
    pub(crate) fn truncate(&mut self, at: u64) {
        // Special case: truncate everything from index 0
        if at == 0 {
            self.key_log_ids.clear();
            return;
        }

        let res = self.key_log_ids.binary_search_by(|log_id| log_id.index().cmp(&at));

        let i = match res {
            Ok(i) => i, // Exact match at entry i
            Err(i) => {
                if i == self.key_log_ids.len() {
                    return; // Beyond all entries, nothing to truncate
                }
                i // Entry i has index > at
            }
        };

        // Entry at position i has index >= at.
        // Determine the start index of entry i's range.
        //
        // The index since which will be removed if no push.
        let end_after_truncate = if i == 0 {
            self.first_index()
        } else {
            self.key_log_ids[i - 1].index() + 1
        };

        // If (at-1) is within entry i's range, keep a truncated version
        if at > end_after_truncate {
            let truncated_leader = self.key_log_ids[i].committed_leader_id().clone();
            self.key_log_ids.truncate(i);
            self.key_log_ids.push(LogIdOf::<C>::new(truncated_leader, at - 1));
        } else {
            // Truncation is before or at entry i's start, remove it entirely
            self.key_log_ids.truncate(i);
        }
    }

    /// Purge log ids up to the log with index `upto_index`, inclusive.
    ///
    /// With last-per-leader storage:
    /// - Set the `purged` field to `upto`
    /// - Remove entries whose last index <= upto.index
    pub(crate) fn purge(&mut self, upto: &LogIdOf<C>) {
        let upto_index = upto.index();

        // When installing snapshot it may need to purge across the `last_log_id`.
        if upto_index >= self.last().next_index() {
            debug_assert!(Some(upto) > self.last());
            self.purged = Some(upto.clone());
            self.key_log_ids.clear();
            return;
        }

        // Already purged further - nothing to do
        if upto_index < self.first_index() {
            return;
        }

        // Find entries that are completely purged (last_index <= upto.index)
        let res = self.key_log_ids.binary_search_by(|log_id| log_id.index().cmp(&upto_index));

        match res {
            Ok(i) => {
                // Exact match: entries 0..=i are purged
                self.key_log_ids = self.key_log_ids.split_off(i + 1);
            }
            Err(i) => {
                // upto.index is between entries: entries 0..i are purged
                self.key_log_ids = self.key_log_ids.split_off(i);
            }
        }

        self.purged = Some(upto.clone());
    }

    // This method is only used in tests
    #[allow(dead_code)]
    pub(crate) fn get(&self, index: u64) -> Option<LogIdOf<C>> {
        self.ref_at(index).map(|r| r.into_log_id())
    }

    /// Get the log id at the specified index in a [`RefLogId`].
    ///
    /// With last-per-leader storage, each entry represents a range:
    /// - Entry at i covers indices from `(key_log_ids[i-1].index + 1)` to `key_log_ids[i].index`
    /// - Entry at 0 covers indices from `(purged.index + 1)` to `key_log_ids[0].index`
    ///
    /// Returns the `purged` log id if the index equals the purged index.
    #[allow(clippy::clone_on_copy)]
    pub(crate) fn ref_at(&self, index: u64) -> Option<RefLogId<'_, C>> {
        // Handle purged range
        // index < next_index() implies purged is Some (otherwise next_index() returns 0)
        if index < self.first_index() {
            let purged = self.purged.as_ref().unwrap();
            return if index == purged.index() {
                Some(purged.to_ref())
            } else {
                None // Index is before the first available log
            };
        }

        let res = self.key_log_ids.binary_search_by(|log_id| log_id.index().cmp(&index));

        // With last-per-leader, find the entry whose range contains the index
        let i = match res {
            Ok(i) => i, // Exact match
            Err(i) => {
                // i is the first entry where last_index > target
                // This entry's leader contains the target index
                if i >= self.key_log_ids.len() {
                    return None; // Index beyond last log
                }
                i
            }
        };

        // Validate: index must be within the range of entry i
        let range_start = if i == 0 {
            self.first_index()
        } else {
            self.key_log_ids[i - 1].index() + 1
        };

        if index < range_start {
            return None;
        }

        Some(RefLogId::new(self.key_log_ids[i].committed_leader_id(), index))
    }

    /// Get the first log id as a `RefLogId`.
    ///
    /// The first log index is `purged.index + 1` (or 0 if nothing purged).
    /// The leader comes from the first entry in `key_log_ids`.
    pub(crate) fn first(&self) -> Option<RefLogId<'_, C>> {
        let first_key = self.key_log_ids.first()?;
        let first_index = self.first_index();
        Some(RefLogId::new(first_key.committed_leader_id(), first_index))
    }

    pub(crate) fn last(&self) -> Option<&LogIdOf<C>> {
        self.key_log_ids.last().or(self.purged.as_ref())
    }

    pub(crate) fn last_ref(&self) -> Option<RefLogId<'_, C>> {
        self.last().map(|x| x.to_ref())
    }

    #[allow(dead_code)]
    pub(crate) fn last_committed_leader_id(&self) -> Option<&CommittedLeaderIdOf<C>> {
        self.last().map(|x| x.committed_leader_id())
    }

    // This method will only be used under feature tokio-rt
    #[cfg_attr(not(feature = "tokio-rt"), allow(dead_code))]
    pub(crate) fn key_log_ids(&self) -> &[LogIdOf<C>] {
        &self.key_log_ids
    }

    /// Returns key log ids appended by the last leader.
    ///
    /// With last-per-leader storage, the first index of the last leader is:
    /// - If there's a previous entry: previous_entry.index + 1
    /// - Otherwise: purged.index + 1 (or 0 if nothing purged)
    ///
    /// If key_log_ids is empty but purged is Some, returns the purged log info.
    ///
    /// Note that the 0-th log does not belong to any leader (but a membership log to initialize a
    /// cluster), but this method does not differentiate between them.
    pub(crate) fn by_last_leader(&self) -> Option<LeaderLogIds<C>> {
        let last = self.last()?;
        let l = self.key_log_ids.len();

        let first_index = if l == 0 {
            // No entries in key_log_ids, last is from purged
            last.index()
        } else if l == 1 {
            // Only one entry: first index is purged.index + 1 or 0
            self.first_index()
        } else {
            // Previous entry's index + 1
            self.key_log_ids[l - 2].index() + 1
        };

        Some(LeaderLogIds::new(
            last.committed_leader_id().clone(),
            first_index,
            last.index(),
        ))
    }
}
