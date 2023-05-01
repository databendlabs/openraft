use crate::LogId;
use crate::LogIdOptionExt;
use crate::NodeId;

/// APIs to get significant log ids reflecting the raft state.
///
/// See: [`log_pointers`](`crate::docs::data::log_pointers`).
pub(crate) trait LogStateReader<NID: NodeId> {
    /// Get previous log id, i.e., the log id at index - 1
    fn prev_log_id(&self, index: u64) -> Option<LogId<NID>> {
        if index == 0 {
            None
        } else {
            self.get_log_id(index - 1)
        }
    }

    /// Return if a log id exists.
    ///
    /// It assumes a committed log will always get positive return value, according to raft spec.
    fn has_log_id(&self, log_id: &LogId<NID>) -> bool {
        if log_id.index < self.committed().next_index() {
            debug_assert!(Some(log_id) <= self.committed());
            return true;
        }

        // The local log id exists at the index and is same as the input.
        if let Some(local) = self.get_log_id(log_id.index) {
            *log_id == local
        } else {
            false
        }
    }

    /// Get the log id at the specified index.
    ///
    /// It will return `last_purged_log_id` if index is at the last purged index.
    /// If the log at the specified index is smaller than `last_purged_log_id`, or greater than
    /// `last_log_id`, it returns None.
    fn get_log_id(&self, index: u64) -> Option<LogId<NID>>;

    /// The last known log id in the store.
    ///
    /// The range of all stored log ids are `(last_purged_log_id(), last_log_id()]`, left open right
    /// close.
    fn last_log_id(&self) -> Option<&LogId<NID>>;

    /// The last known committed log id, i.e., the id of the log that is accepted by a quorum of
    /// voters.
    fn committed(&self) -> Option<&LogId<NID>>;

    /// The last known applied log id, i.e., the id of the log that is applied to state machine.
    ///
    /// This is actually happened io-state which might fall behind committed log id.
    fn io_applied(&self) -> Option<&LogId<NID>>;

    /// The last known purged log id, inclusive.
    ///
    /// This is actually purged log id from storage.
    fn io_purged(&self) -> Option<&LogId<NID>>;

    /// Return the last log id the snapshot includes.
    fn snapshot_last_log_id(&self) -> Option<&LogId<NID>>;

    /// Return the log id it wants to purge up to.
    ///
    /// Logs may not be able to be purged at once because they are in use by replication tasks.
    fn purge_upto(&self) -> Option<&LogId<NID>>;

    /// The greatest log id that has been purged after being applied to state machine, i.e., the
    /// oldest known log id.
    ///
    /// The range of log entries that exist in storage is `(last_purged_log_id(), last_log_id()]`,
    /// left open and right close.
    ///
    /// `last_purged_log_id == last_log_id` means there is no log entry in the storage.
    fn last_purged_log_id(&self) -> Option<&LogId<NID>>;
}
