use crate::RaftTypeConfig;
use crate::log_id::option_raft_log_id_ext::OptionRaftLogIdExt;
use crate::log_id::option_ref_log_id_ext::OptionRefLogIdExt;
use crate::log_id::raft_log_id::RaftLogId;
use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::log_id::ref_log_id::RefLogId;
use crate::type_config::alias::LogIdOf;

/// APIs to get significant log ids reflecting the raft state.
///
/// See: [`log_pointers`](`crate::docs::data::log_pointers`).
pub(crate) trait LogStateReader<C>
where C: RaftTypeConfig
{
    /// Get previous log id, i.e., the log id at index - 1
    fn prev_log_id(&self, index: u64) -> Option<LogIdOf<C>> {
        if index == 0 { None } else { self.get_log_id(index - 1) }
    }

    /// Return if a log id exists.
    ///
    /// It assumes a committed log will always get positive return value, according to raft spec.
    fn has_log_id(&self, log_id: impl RaftLogId<C>) -> bool {
        if log_id.index() < self.committed().next_index() {
            debug_assert!(Some(log_id.to_ref()) <= self.committed().to_ref());
            return true;
        }

        // The local log id exists at the index and is same as the input.
        if let Some(local) = self.ref_log_id(log_id.index()) {
            log_id.to_ref() == local
        } else {
            false
        }
    }

    fn get_log_id(&self, index: u64) -> Option<LogIdOf<C>> {
        self.ref_log_id(index).to_log_id()
    }

    /// Get the log id at the specified index.
    ///
    /// It will return `last_purged_log_id` if index is at the last purged index.
    /// If the log at the specified index is smaller than `last_purged_log_id`, or greater than
    /// `last_log_id`, it returns None.
    fn ref_log_id(&self, index: u64) -> Option<RefLogId<'_, C>>;

    /// The last known log id in the store.
    ///
    /// The range of all stored log ids are `(last_purged_log_id(), last_log_id()]`, left open right
    /// close.
    fn last_log_id(&self) -> Option<&LogIdOf<C>>;

    /// The last known committed log id, i.e., the id of the log that is accepted by a quorum of
    /// voters.
    fn committed(&self) -> Option<&LogIdOf<C>>;

    /// The last known applied log id, i.e., the id of the log that is applied to state machine.
    ///
    /// This is actually happened io-state which might fall behind committed log id.
    fn io_applied(&self) -> Option<&LogIdOf<C>>;

    /// The last log id in the last persisted snapshot.
    ///
    /// This is actually happened io-state which might fall behind `Self::snapshot_last_log_id()`.
    fn io_snapshot_last_log_id(&self) -> Option<&LogIdOf<C>>;

    /// The last known purged log id, inclusive.
    ///
    /// This is actually purged log id from storage.
    fn io_purged(&self) -> Option<&LogIdOf<C>>;

    /// Return the last log id the snapshot includes.
    fn snapshot_last_log_id(&self) -> Option<&LogIdOf<C>>;

    /// Return the log id it wants to purge up to.
    ///
    /// Logs may not be able to be purged at once because they are in use by replication tasks.
    fn purge_upto(&self) -> Option<&LogIdOf<C>>;

    /// The greatest log id that has been purged after being applied to state machine, i.e., the
    /// oldest known log id.
    ///
    /// The range of log entries that exist in storage is `(last_purged_log_id(), last_log_id()]`,
    /// left open and right close.
    ///
    /// `last_purged_log_id == last_log_id` means there is no log entry in the storage.
    fn last_purged_log_id(&self) -> Option<&LogIdOf<C>>;
}
