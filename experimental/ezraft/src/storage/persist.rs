//! What the framework asks storage to write

use crate::app::EzApp;
use crate::entry::EzEntry;
use crate::meta::EzMeta;
use crate::snapshot::EzSnapshot;

/// Persistence operation
///
/// Each variant represents one atomic operation that should be persisted to disk.
/// The framework calls [`crate::EzStorage::persist`] with these operations.
#[derive(Debug, derive_more::Display)]
pub enum Persist<T>
where T: EzApp
{
    /// Update Raft metadata (term, vote, log positions)
    #[display("Meta")]
    Meta(EzMeta),

    /// Write a log entry
    ///
    /// Entries arrive in index order and never target an index that is already present: the
    /// framework deletes conflicting entries with [`Persist::DeleteLogs`] first.
    #[display("LogEntry")]
    LogEntry(EzEntry<T>),

    /// Write a complete snapshot, replacing the previous one
    #[display("Snapshot")]
    Snapshot(EzSnapshot),

    /// Delete every log entry in the half-open index range `[from, to)`
    ///
    /// Both reasons a log shrinks arrive here: a tail range (`from..u64::MAX`) deletes entries
    /// that conflict with the leader's log, and a head range (`0..n`) compacts entries already
    /// covered by a snapshot - the only signal that lets storage reclaim space. The implementor
    /// treats both the same: the entries are gone and must not be returned by
    /// [`crate::EzStorage::read_logs`] again.
    #[display("DeleteLogs({from}..{to})")]
    DeleteLogs {
        /// First deleted index
        from: u64,
        /// One past the last deleted index
        to: u64,
    },
}
