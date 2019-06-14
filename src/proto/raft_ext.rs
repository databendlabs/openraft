use super::raft::{entry, Entry, EntrySnapshotPointer};

impl Entry {
    /// Create a new log entry which is a Snapshot pointer.
    pub fn new_snapshot_pointer(term: u64, index: u64, path: String) -> Self {
        let entry_type = Some(entry::EntryType::SnapshotPointer(EntrySnapshotPointer{path}));
        Entry{term, index, entry_type}
    }
}
