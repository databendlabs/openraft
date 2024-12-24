use std::fmt::Debug;
use std::ops::RangeBounds;
use std::ops::RangeInclusive;

use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::engine::LogIdList;
use crate::LogId;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Vote;
/// A trait defining the interface for a Raft log subsystem.
///
/// This interface is accessed read-only by replication sub task: `ReplicationCore`.
///
/// A log reader must also be able to read the last saved vote by [`RaftLogStorage::save_vote`],
/// See: [log-stream](`crate::docs::protocol::replication::log_stream`).
///
/// Typically, the log reader implementation as such will be hidden behind an `Arc<T>` and
/// this interface implemented on the `Arc<T>`. It can be co-implemented with [`RaftLogStorage`]
/// interface on the same cloneable object, if the underlying state machine is anyway synchronized.
///
/// [`RaftLogStorage`]: crate::storage::RaftLogStorage
/// [`RaftLogStorage::save_vote`]: crate::storage::RaftLogStorage::save_vote
#[add_async_trait]
pub trait RaftLogReader<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Get a series of log entries from storage.
    ///
    /// ### Correctness requirements
    ///
    /// - The absence of an entry is tolerated only at the beginning or end of the range. Missing
    ///   entries within the range (i.e., holes) are not permitted and should result in a
    ///   `StorageError`.
    ///
    /// - The read operation must be transactional. That is, it should not reflect any state changes
    ///   that occur after the read operation has commenced.
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<C::Entry>, StorageError<C>>;

    /// Return the last saved vote by [`RaftLogStorage::save_vote`].
    ///
    /// A log reader must also be able to read the last saved vote by [`RaftLogStorage::save_vote`],
    /// See: [log-stream](`crate::docs::protocol::replication::log_stream`)
    ///
    /// [`RaftLogStorage::save_vote`]: crate::storage::RaftLogStorage::save_vote
    async fn read_vote(&mut self) -> Result<Option<Vote<C>>, StorageError<C>>;

    /// Returns log entries within range `[start, end)`, `end` is exclusive,
    /// potentially limited by implementation-defined constraints.
    ///
    /// If the specified range is too large, the implementation may return only the first few log
    /// entries to ensure the result is not excessively large.
    ///
    /// It must not return empty result if the input range is not empty.
    ///
    /// The default implementation just returns the full range of log entries.
    #[since(version = "0.10.0")]
    async fn limited_get_log_entries(&mut self, start: u64, end: u64) -> Result<Vec<C::Entry>, StorageError<C>> {
        self.try_get_log_entries(start..end).await
    }

    /// Retrieves a list of key log ids that mark the beginning of each Leader.
    ///
    /// This method returns log entries that represent leadership transitions in the log history,
    /// including:
    /// - The first log entry in the storage (regardless of Leader);
    /// - The first log entry from each Leader;
    /// - The last log entry in the storage (regardless of Leader);
    ///
    /// # Example
    ///
    /// Given:
    /// Log entries: `[(2,2), (2,3), (5,4), (5,5)]` (format: `(term, index)`)
    ///
    /// Returns: `[(2,2), (5,4), (5,5)]`
    ///
    /// # Usage
    ///
    /// This method is called only during node startup to build an initial log index.
    ///
    /// # Implementation Notes
    ///
    /// - Optional method: If your [`RaftLogStorage`] implementation doesn't maintain this
    ///   information, do not implement it and use the default implementation.
    /// - Default implementation: Uses a binary search algorithm to find key log entries
    /// - Time complexity: `O(k * log(n))` where:
    ///   - `k` = average number of unique Leaders
    ///   - `n` = average number of logs per Leader
    ///
    /// # Arguments
    ///
    /// - `range`: range of the log id to return, inclusive. Such as `(1, 10)..=(2, 20)`.
    ///
    /// # Returns
    ///
    /// Returns a vector of log entries marking leadership transitions and boundaries.
    ///
    /// [`RaftLogStorage`]: crate::storage::RaftLogStorage
    #[since(version = "0.10.0")]
    async fn get_key_log_ids(&mut self, range: RangeInclusive<LogId<C>>) -> Result<Vec<LogId<C>>, StorageError<C>> {
        LogIdList::get_key_log_ids(range, self).await
    }
}
