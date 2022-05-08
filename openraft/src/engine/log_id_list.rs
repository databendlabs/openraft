use crate::raft_types::RaftLogId;
use crate::LogId;
use crate::NodeId;

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

    /// Append a new `log_id`.
    ///
    /// The log id to append does not have to be the next to the last one in `key_log_ids`.
    /// In such case, it is meant to append a list of log ids.
    ///
    /// NOTE: The last two in `key_log_ids` may be with the same `leader_id`, because `last_log_id` always present in
    /// `log_ids`.
    pub(crate) fn append(&mut self, new_log_id: LogId<NID>) {
        let l = self.key_log_ids.len();
        if l == 0 {
            self.key_log_ids.push(new_log_id);
            return;
        }

        // l >= 1

        assert!(new_log_id > self.key_log_ids[l - 1]);

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

    /// Purge log ids upto the log with index `upto_index`, inclusive.
    #[allow(dead_code)]
    pub(crate) fn purge(&mut self, upto: &LogId<NID>) {
        // When installing  snapshot it may need to purge across the `last_log_id`.
        if upto.index > self.key_log_ids[self.key_log_ids.len() - 1].index {
            assert!(upto > &self.key_log_ids[self.key_log_ids.len() - 1]);
            self.key_log_ids = vec![*upto];
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
    #[allow(dead_code)]
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

    pub(crate) fn key_log_ids(&self) -> &[LogId<NID>] {
        &self.key_log_ids
    }
}
