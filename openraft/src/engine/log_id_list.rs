use crate::log_id::RaftLogId;
use crate::storage::RaftLogReaderExt;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::NodeId;
use crate::RaftTypeConfig;
use crate::StorageError;

/// Efficient storage for log ids.
///
/// It stores only the ids of log that have a new leader_id. And the `last_log_id` at the end.
/// I.e., the oldest log id belonging to every leader.
///
/// If it is not empty, the first one is `last_purged_log_id` and the last one is `last_log_id`.
/// The last one may have the same leader id as the second last one.
#[derive(Default, Debug, Clone)]
#[derive(PartialEq, Eq)]
pub struct LogIdList<NID: NodeId> {
    key_log_ids: Vec<LogId<NID>>,
}

impl<NID> LogIdList<NID>
where NID: NodeId
{
    /// Load all log ids that are the first one proposed by a leader.
    ///
    /// E.g., log ids with the same leader id will be got rid of, except the smallest.
    /// The `last_log_id` will always present at the end, to simplify searching.
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
    pub(crate) async fn load_log_ids<C, LRX>(
        last_purged_log_id: Option<LogId<NID>>,
        last_log_id: Option<LogId<NID>>,
        sto: &mut LRX,
    ) -> Result<LogIdList<NID>, StorageError<NID>>
    where
        C: RaftTypeConfig<NodeId = NID>,
        LRX: RaftLogReaderExt<C>,
    {
        let mut res = vec![];

        let last = match last_log_id {
            None => return Ok(LogIdList::new(res)),
            Some(x) => x,
        };
        let first = match last_purged_log_id {
            None => sto.get_log_id(0).await?,
            Some(x) => x,
        };

        // Recursion stack
        let mut stack = vec![(first, last)];

        loop {
            let (first, last) = match stack.pop() {
                None => {
                    break;
                }
                Some(x) => x,
            };

            // Case AA
            if first.leader_id == last.leader_id {
                if res.last().map(|x| x.leader_id) < Some(first.leader_id) {
                    res.push(first);
                }
                continue;
            }

            // Two adjacent logs with different leader_id, no need to binary search
            if first.index + 1 == last.index {
                if res.last().map(|x| x.leader_id) < Some(first.leader_id) {
                    res.push(first);
                }
                res.push(last);
                continue;
            }

            let mid = sto.get_log_id((first.index + last.index) / 2).await?;

            if first.leader_id == mid.leader_id {
                // Case AAC
                if res.last().map(|x| x.leader_id) < Some(first.leader_id) {
                    res.push(first);
                }
                stack.push((mid, last));
            } else if mid.leader_id == last.leader_id {
                // Case ACC
                stack.push((first, mid));
            } else {
                // Case ABC
                // first.leader_id < mid_log_id.leader_id < last.leader_id
                // Deal with (first, mid) then (mid, last)
                stack.push((mid, last));
                stack.push((first, mid));
            }
        }

        if res.last() != Some(&last) {
            res.push(last);
        }

        Ok(LogIdList::new(res))
    }
}

impl<NID: NodeId> LogIdList<NID> {
    pub fn new(key_log_ids: impl IntoIterator<Item = LogId<NID>>) -> Self {
        Self {
            key_log_ids: key_log_ids.into_iter().collect(),
        }
    }

    /// Extends a list of `log_id` that are proposed by a same leader.
    ///
    /// The log ids in the input has to be continuous.
    pub(crate) fn extend_from_same_leader<'a, LID: RaftLogId<NID> + 'a>(&mut self, new_ids: &[LID]) {
        if let Some(first) = new_ids.first() {
            let first_id = first.get_log_id();
            self.append(*first_id);

            if let Some(last) = new_ids.last() {
                let last_id = last.get_log_id();
                assert_eq!(last_id.leader_id, first_id.leader_id);

                if last_id != first_id {
                    self.append(*last_id);
                }
            }
        }
    }

    /// Extends a list of `log_id`.
    #[allow(dead_code)]
    pub(crate) fn extend<'a, LID: RaftLogId<NID> + 'a>(&mut self, new_ids: &[LID]) {
        let mut prev = self.last().map(|x| x.leader_id);

        for x in new_ids.iter() {
            let log_id = x.get_log_id();

            if prev != Some(log_id.leader_id) {
                self.append(*log_id);

                prev = Some(log_id.leader_id);
            }
        }

        if let Some(last) = new_ids.last() {
            let log_id = last.get_log_id();

            if self.last() != Some(log_id) {
                self.append(*log_id);
            }
        }
    }

    /// Append a new `log_id`.
    ///
    /// The log id to append does not have to be the next to the last one in `key_log_ids`.
    /// In such case, it fills the gap at index `i` with `LogId{leader_id: prev_log_id.leader,
    /// index: i}`.
    ///
    /// NOTE: The last two in `key_log_ids` may be with the same `leader_id`, because `last_log_id`
    /// always present in `log_ids`.
    pub(crate) fn append(&mut self, new_log_id: LogId<NID>) {
        let l = self.key_log_ids.len();
        if l == 0 {
            self.key_log_ids.push(new_log_id);
            return;
        }

        // l >= 1

        debug_assert!(new_log_id > self.key_log_ids[l - 1]);

        if l == 1 {
            self.key_log_ids.push(new_log_id);
            return;
        }

        // l >= 2

        let last = self.key_log_ids[l - 1];

        if self.key_log_ids.get(l - 2).map(|x| x.leader_id) == Some(last.leader_id) {
            // Replace the **last log id**.
            self.key_log_ids[l - 1] = new_log_id;
            return;
        }

        // The last one is an initial log entry of a leader.
        // Add a **last log id** with the same leader id.

        self.key_log_ids.push(new_log_id);
    }

    /// Delete log ids from `at`, inclusive.
    #[allow(dead_code)]
    pub(crate) fn truncate(&mut self, at: u64) {
        let res = self.key_log_ids.binary_search_by(|log_id| log_id.index.cmp(&at));

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
            let (last_leader_id, last_index) = (last.leader_id, last.index);
            if last_index < at - 1 {
                self.append(LogId::new(last_leader_id, at - 1));
            }
        }
    }

    /// Purge log ids upto the log with index `upto_index`, inclusive.
    #[allow(dead_code)]
    pub(crate) fn purge(&mut self, upto: &LogId<NID>) {
        let last = self.last().cloned();

        // When installing  snapshot it may need to purge across the `last_log_id`.
        if upto.index >= last.next_index() {
            debug_assert!(Some(upto) > self.last());
            self.key_log_ids = vec![*upto];
            return;
        }

        if upto.index < self.key_log_ids[0].index {
            return;
        }

        let res = self.key_log_ids.binary_search_by(|log_id| log_id.index.cmp(&upto.index));

        match res {
            Ok(i) => {
                if i > 0 {
                    self.key_log_ids = self.key_log_ids.split_off(i)
                }
            }
            Err(i) => {
                self.key_log_ids = self.key_log_ids.split_off(i - 1);
                self.key_log_ids[0].index = upto.index;
            }
        }
    }

    /// Get the log id at the specified index.
    ///
    /// It will return `last_purged_log_id` if index is at the last purged index.
    pub(crate) fn get(&self, index: u64) -> Option<LogId<NID>> {
        let res = self.key_log_ids.binary_search_by(|log_id| log_id.index.cmp(&index));

        match res {
            Ok(i) => Some(LogId::new(self.key_log_ids[i].leader_id, index)),
            Err(i) => {
                if i == 0 || i == self.key_log_ids.len() {
                    None
                } else {
                    Some(LogId::new(self.key_log_ids[i - 1].leader_id, index))
                }
            }
        }
    }

    pub(crate) fn first(&self) -> Option<&LogId<NID>> {
        self.key_log_ids.first()
    }

    pub(crate) fn last(&self) -> Option<&LogId<NID>> {
        self.key_log_ids.last()
    }

    pub(crate) fn key_log_ids(&self) -> &[LogId<NID>] {
        &self.key_log_ids
    }

    /// Returns key log ids appended by the last leader.
    ///
    /// Note that the 0-th log does not belong to any leader(but a membership log to initialize a
    /// cluster) but this method does not differentiate between them.
    pub(crate) fn by_last_leader(&self) -> &[LogId<NID>] {
        let ks = &self.key_log_ids;
        let l = ks.len();
        if l < 2 {
            return ks;
        }

        // There are at most two(adjacent) key log ids with the same leader_id
        if ks[l - 1].leader_id() == ks[l - 2].leader_id() {
            &ks[l - 2..]
        } else {
            &ks[l - 1..]
        }
    }
}
