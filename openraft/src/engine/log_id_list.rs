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
/// It stores only the ids of logs that have a new leader_id. And the `last_log_id` at the end.
/// I.e., the oldest log id belonging to every leader.
///
/// If it is not empty, the first one is `last_purged_log_id` and the last one is `last_log_id`.
/// The last one may have the same leader id as the second last one.
#[derive(Default, Debug, Clone)]
#[derive(PartialEq, Eq)]
pub struct LogIdList<C>
where C: RaftTypeConfig
{
    key_log_ids: Vec<LogIdOf<C>>,
}

impl<C> LogIdList<C>
where C: RaftTypeConfig
{
    /// Load all log ids that are the first one proposed by a leader.
    ///
    /// E.g., log ids with the same leader id will be discarded, except the smallest.
    /// The `last_log_id` will always be present at the end to simplify searching.
    ///
    /// Given an example with the logs `[(2,2),(2,3),(5,4),(5,5)]`, and the `last_purged_log_id` is
    /// (1,1). This function returns `[(1,1),(2,2),(5,4),(5,5)]`.
    ///
    /// It adopts a modified binary-search algo.
    /// ```text
    /// input:
    /// A---------------C
    ///
    /// load the mid log-id, then compare the first, the middle, and the last:
    ///
    /// A---------------A : push_res(A);
    /// A-------A-------C : push_res(A); find(A,C) // both find `A`, need to de-dup
    /// A-------B-------C : find(A,B); find(B,C)   // both find `B`, need to de-dup
    /// A-------C-------C : find(A,C)
    /// ```
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
        let mut stack = vec![(first, last.clone())];

        loop {
            let (first, last) = match stack.pop() {
                None => {
                    break;
                }
                Some(x) => x,
            };

            // Case AA
            if first.committed_leader_id() == last.committed_leader_id() {
                if res.last().map(|x| x.committed_leader_id()) < Some(first.committed_leader_id()) {
                    res.push(first);
                }
                continue;
            }

            // Two adjacent logs with different leader_id, no need to binary search
            if first.index() + 1 == last.index() {
                if res.last().committed_leader_id() < Some(first.committed_leader_id()) {
                    res.push(first);
                }
                res.push(last);
                continue;
            }

            let mid_index = (first.index() + last.index()) / 2;
            let mid = sto.get_log_id(mid_index).await?;

            if first.committed_leader_id() == mid.committed_leader_id() {
                // Case AAC
                if res.last().committed_leader_id() < Some(first.committed_leader_id()) {
                    res.push(first);
                }
                stack.push((mid, last));
            } else if mid.committed_leader_id() == last.committed_leader_id() {
                // Case ACC
                stack.push((first, mid));
            } else {
                // Case ABC
                // first.leader_id < mid_log_id.leader_id < last.leader_id
                // Deal with (first, mid) then (mid, last)
                stack.push((mid.clone(), last));
                stack.push((first, mid));
            }
        }

        if res.last() != Some(&last) {
            res.push(last);
        }

        Ok(res)
    }
}

impl<C> LogIdList<C>
where C: RaftTypeConfig
{
    /// Create a new `LogIdList`.
    ///
    /// It stores the last purged log id, and a series of key log ids.
    pub fn new(key_log_ids: impl IntoIterator<Item = LogIdOf<C>>) -> Self {
        Self {
            key_log_ids: key_log_ids.into_iter().collect(),
        }
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

    /// Extends a list of `log_id`.
    pub(crate) fn extend<LID, I>(&mut self, new_ids: I)
    where
        LID: RaftLogId<C>,
        I: IntoIterator<Item = LID>,
        <I as IntoIterator>::IntoIter: ExactSizeIterator,
    {
        let it = new_ids.into_iter();
        let len = it.len();

        for (i, log_id) in it.enumerate() {
            if self.last_committed_leader_id() != Some(log_id.committed_leader_id()) {
                self.append(log_id.to_log_id());
            }

            #[allow(clippy::collapsible_if)]
            if i == len - 1 {
                if self.last_ref() != Some(log_id.to_ref()) {
                    self.append(log_id.to_log_id());
                }
            }
        }
    }

    /// Append a new `log_id`.
    ///
    /// The log id to append does not have to be the next to the last one in `key_log_ids`.
    /// In such a case it fills the gap at index `i` with `LogId{leader_id: prev_log_id.leader,
    /// index: i}`.
    ///
    /// NOTE: The last two in `key_log_ids` may be with the same `leader_id`, because `last_log_id`
    /// is always present in `log_ids`.
    pub(crate) fn append(&mut self, new_log_id: LogIdOf<C>) {
        let l = self.key_log_ids.len();
        if l == 0 {
            self.key_log_ids.push(new_log_id);
            return;
        }

        // l >= 1

        debug_assert!(
            new_log_id > self.key_log_ids[l - 1],
            "new_log_id: {}, last: {}",
            new_log_id,
            self.key_log_ids[l - 1]
        );

        if l == 1 {
            self.key_log_ids.push(new_log_id);
            return;
        }

        // l >= 2

        let last = &self.key_log_ids[l - 1];

        if self.key_log_ids.get(l - 2).map(|x| x.committed_leader_id()) == Some(last.committed_leader_id()) {
            // Replace the **last log id**.
            self.key_log_ids[l - 1] = new_log_id;
            return;
        }

        // The last one is an initial log entry of a leader.
        // Add a **last log id** with the same leader id.

        self.key_log_ids.push(new_log_id);
    }

    /// Delete log ids from `at`, inclusive.
    // leader_id: Copy is feature gated
    #[allow(clippy::clone_on_copy)]
    pub(crate) fn truncate(&mut self, at: u64) {
        let res = self.key_log_ids.binary_search_by(|log_id| log_id.index().cmp(&at));

        let i = match res {
            Ok(i) => i,
            Err(i) => {
                if i == self.key_log_ids.len() {
                    return;
                }
                i
            }
        };

        self.key_log_ids.truncate(i);

        // Add key log id if there is a gap between last.index and at - 1.
        let last = self.key_log_ids.last();
        if let Some(last) = last {
            let (last_leader_id, last_index) = (last.committed_leader_id().clone(), last.index());
            if last_index < at - 1 {
                self.append(LogIdOf::<C>::new(last_leader_id, at - 1));
            }
        }
    }

    /// Purge log ids up to the log with index `upto_index`, inclusive.
    pub(crate) fn purge(&mut self, upto: &LogIdOf<C>) {
        let last = self.last().cloned();

        // When installing snapshot it may need to purge across the `last_log_id`.
        if upto.index() >= last.next_index() {
            debug_assert!(Some(upto) > self.last());
            self.key_log_ids = vec![upto.clone()];
            return;
        }

        if upto.index() < self.key_log_ids[0].index() {
            return;
        }

        let res = self.key_log_ids.binary_search_by(|log_id| log_id.index().cmp(&upto.index()));

        match res {
            Ok(i) => {
                if i > 0 {
                    self.key_log_ids = self.key_log_ids.split_off(i)
                }
            }
            Err(i) => {
                self.key_log_ids = self.key_log_ids.split_off(i - 1);
                self.key_log_ids[0].index = upto.index();
            }
        }
    }

    // This method is only used in tests
    #[allow(dead_code)]
    pub(crate) fn get(&self, index: u64) -> Option<LogIdOf<C>> {
        self.ref_at(index).map(|r| r.into_log_id())
    }

    /// Get the log id at the specified index in a [`RefLogId`].
    ///
    /// It will return `last_purged_log_id` if the index is at the last purged index.
    #[allow(clippy::clone_on_copy)]
    pub(crate) fn ref_at(&self, index: u64) -> Option<RefLogId<'_, C>> {
        let res = self.key_log_ids.binary_search_by(|log_id| log_id.index().cmp(&index));

        // Index of the leadership change point that covers the target index.
        // It points to either:
        // - The exact matching log entry if found, or
        // - The most recent change point before the target index
        let change_point = match res {
            Ok(i) => i,
            Err(i) => {
                // i - 1 is the last one that is smaller than the input.
                if i == 0 || i == self.key_log_ids.len() {
                    return None;
                } else {
                    i - 1
                }
            }
        };

        Some(RefLogId::new(
            self.key_log_ids[change_point].committed_leader_id(),
            index,
        ))
    }

    pub(crate) fn first(&self) -> Option<&LogIdOf<C>> {
        self.key_log_ids.first()
    }

    pub(crate) fn last(&self) -> Option<&LogIdOf<C>> {
        self.key_log_ids.last()
    }

    pub(crate) fn last_ref(&self) -> Option<RefLogId<'_, C>> {
        self.last().map(|x| x.to_ref())
    }

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
    /// Note that the 0-th log does not belong to any leader (but a membership log to initialize a
    /// cluster), but this method does not differentiate between them.
    #[allow(dead_code)]
    pub(crate) fn by_last_leader(&self) -> LeaderLogIds<C> {
        let ks = &self.key_log_ids;
        let l = ks.len();
        if l < 2 {
            let last = self.last();
            return LeaderLogIds::new(last.map(|x| x.clone()..=x.clone()));
        }

        // There are at most two(adjacent) key log ids with the same leader_id
        if ks[l - 1].committed_leader_id() == ks[l - 2].committed_leader_id() {
            LeaderLogIds::new_start_end(ks[l - 2].clone(), ks[l - 1].clone())
        } else {
            let last = self.last().cloned().unwrap();
            LeaderLogIds::new_single(last)
        }
    }
}
