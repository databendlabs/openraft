use std::fmt::Debug;
use std::io;
use std::ops::RangeBounds;
use std::ops::RangeInclusive;

use futures::Stream;
use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::base::BoxStream;
use crate::engine::LogIdList;
use crate::error::LeaderChanged;
use crate::type_config::alias::EntryOf;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;
use crate::vote::RaftVote;

/// Error that can occur when reading log entries from a leader-bounded stream.
///
/// This error is returned by [`RaftLogReader::leader_bounded_stream`] when either:
/// - The leader has changed ([`LeaderChanged`])
/// - An I/O error occurred during log reading
#[derive(Debug, thiserror::Error)]
pub enum LeaderBoundedStreamError<C>
where C: RaftTypeConfig
{
    /// The leader has changed, making the stream invalid.
    #[error(transparent)]
    LeaderChanged(#[from] LeaderChanged<C>),

    /// An I/O error occurred while reading log entries.
    #[error(transparent)]
    IoError(#[from] io::Error),
}

/// Result type for [`RaftLogReader::leader_bounded_stream`] stream items.
pub type LeaderBoundedStreamResult<C> = Result<EntryOf<C>, LeaderBoundedStreamError<C>>;

/// Result type for [`RaftLogReader::entries_stream`] stream items.
pub type EntriesStreamResult<C> = Result<EntryOf<C>, io::Error>;

/// A trait defining the interface for a Raft log subsystem.
///
/// This interface is accessed read-only by replication sub-task: `ReplicationCore`.
///
/// A log reader must also be able to read the last saved vote by [`RaftLogStorage::save_vote`],
/// See: [log-stream](`crate::docs::protocol::replication::log_stream`).
///
/// Typically, the log reader implementation as such will be hidden behind an `Arc<T>` and
/// this interface implemented on the `Arc<T>`. It can be co-implemented with [`RaftLogStorage`]
/// interface on the same cloneable object if the underlying state machine is anyway synchronized.
///
/// [`RaftLogStorage`]: crate::storage::RaftLogStorage
/// [`RaftLogStorage::save_vote`]: crate::storage::RaftLogStorage::save_vote
#[add_async_trait]
pub trait RaftLogReader<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Read log entries as a stream, conditional on vote state.
    ///
    /// Returns log entries within the specified range as a stream, but only if:
    /// - The current vote belongs to the given `leader`
    /// - The vote is committed (i.e., [`Self::read_vote`]`.await?.is_committed()`)
    ///
    /// # Important
    ///
    /// The stream must stop yielding entries immediately and return [`LeaderChanged`] error if the
    /// leader changes during iteration, as log truncation may occur and entries may no longer
    /// be consecutive.
    ///
    /// # Arguments
    ///
    /// * `leader` - The expected leader ID. Entries are only returned if the vote belongs to this
    ///   leader.
    /// * `range` - The range of log indices to read.
    ///
    /// # Returns
    ///
    /// Returns a stream of log entries wrapped in `Result`. If the vote doesn't match the leader,
    /// isn't committed, or changes during the read, returns a stream yielding a single
    /// [`LeaderChanged`] error.
    ///
    /// # Default Implementation
    ///
    /// The default implementation is a very simple wrapper of [`Self::try_get_log_entries`], and
    /// application should implement it for better performance.
    ///
    /// The default impl:
    /// 1. Checks if the stored vote matches the given leader and is committed
    /// 2. If conditions are met, calls [`Self::try_get_log_entries`] to get all entries
    /// 3. Verifies the vote hasn't changed after reading entries
    /// 4. Returns stream with [`LeaderChanged`] error if vote doesn't match or changed, otherwise
    ///    returns entries as stream
    #[since(version = "0.10.0")]
    async fn leader_bounded_stream<RB>(
        &mut self,
        leader: LeaderIdOf<C>,
        range: RB,
    ) -> impl Stream<Item = LeaderBoundedStreamResult<C>> + OptionalSend
    where
        RB: RangeBounds<u64> + Clone + Debug + OptionalSend,
    {
        // TODO: complete the test that ensures when the vote is changed, stream should be stopped.

        use futures::stream;

        let changed_err = |leader, vote| {
            let err = LeaderChanged::new(leader, vote);
            LeaderBoundedStreamError::LeaderChanged(err)
        };

        let fu = async move {
            let vote = self.read_vote().await?;

            let Some(vote) = vote else {
                return Ok::<BoxStream<_>, _>(Box::pin(stream::iter([Err(changed_err(leader, None))])));
            };

            if vote.leader_id() == Some(&leader) && vote.is_committed() {
                // valid vote, continue
            } else {
                return Ok::<BoxStream<_>, _>(Box::pin(stream::iter([Err(changed_err(leader, Some(vote)))])));
            }

            let entries = self.try_get_log_entries(range).await?;

            let current_vote = self.read_vote().await?;

            if current_vote.as_ref() != Some(&vote) {
                // Vote has changed, return empty stream
                return Ok::<BoxStream<_>, _>(Box::pin(stream::iter([Err(changed_err(leader, current_vote))])));
            }

            // Vote unchanged
            let stream = stream::iter(entries.into_iter().map(Ok));
            Ok::<BoxStream<_>, io::Error>(Box::pin(stream))
        };

        match fu.await {
            Ok(strm) => strm,
            Err(io_err) => Box::pin(stream::iter([Err(LeaderBoundedStreamError::IoError(io_err))])),
        }
    }

    /// Read log entries as a stream without leader validation.
    ///
    /// Returns log entries within the specified range as a stream. Unlike
    /// [`leader_bounded_stream`](Self::leader_bounded_stream), this method does not check vote
    /// state or leader validity.
    ///
    /// This method is primarily used for reading committed log entries that will no longer be
    /// changed, such as when applying logs to the state machine. It can also be used during
    /// initialization or when the leader context is not relevant.
    ///
    /// # Arguments
    ///
    /// * `range` - The range of log indices to read.
    ///
    /// # Returns
    ///
    /// Returns a stream of log entries wrapped in `Result`. I/O errors during reading are
    /// propagated through the stream.
    ///
    /// # Default Implementation
    ///
    /// The default implementation calls [`Self::try_get_log_entries`] to get all entries and
    /// converts them to a stream. Applications may implement this for better performance, such as
    /// streaming entries incrementally.
    #[since(version = "0.10.0")]
    async fn entries_stream<RB>(&mut self, range: RB) -> impl Stream<Item = EntriesStreamResult<C>> + OptionalSend
    where RB: RangeBounds<u64> + Clone + Debug + OptionalSend {
        use futures::stream;

        let fu = async move {
            let entries = self.try_get_log_entries(range).await?;

            let stream = stream::iter(entries.into_iter().map(Ok));
            Ok::<BoxStream<_>, io::Error>(Box::pin(stream))
        };

        match fu.await {
            Ok(strm) => strm,
            Err(io_err) => Box::pin(stream::iter([Err(io_err)])),
        }
    }

    /// Get a series of log entries from storage.
    ///
    /// This method retrieves log entries within the specified range. The range is defined by
    /// any type that implements `RangeBounds<u64>`, such as `start..end` or `start..=end`.
    ///
    /// ### Correctness requirements
    ///
    /// - All log entries in the range must be returned. Unlike
    ///   [`limited_get_log_entries()`](Self::limited_get_log_entries), which may return only the
    ///   first several log entries.
    ///
    /// - If the log doesn't contain all the requested entries, return the existing entries. The
    ///   absence of an entry is tolerated only at the beginning or end of the range. Missing
    ///   entries within the range (i.e., holes) are not permitted and should result in an error.
    ///
    /// - The read operation must be atomic. That is, it should not reflect any state changes that
    ///   occur after the read operation has started.
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<C::Entry>, io::Error>;

    /// Return the last saved vote by [`RaftLogStorage::save_vote`].
    ///
    /// A log reader must also be able to read the last saved vote by [`RaftLogStorage::save_vote`],
    /// See: [log-stream](`crate::docs::protocol::replication::log_stream`)
    ///
    /// [`RaftLogStorage::save_vote`]: crate::storage::RaftLogStorage::save_vote
    async fn read_vote(&mut self) -> Result<Option<VoteOf<C>>, io::Error>;

    /// Returns log entries within range `[start, end)`, `end` is exclusive,
    /// potentially limited by implementation-defined constraints.
    ///
    /// If the specified range is too large, the implementation may return only the first few log
    /// entries to ensure the result is not excessively large.
    ///
    /// It must not return an empty result if the input range is not empty.
    ///
    /// The default implementation just returns the full range of log entries.
    #[since(version = "0.10.0")]
    async fn limited_get_log_entries(&mut self, start: u64, end: u64) -> Result<Vec<C::Entry>, io::Error> {
        self.try_get_log_entries(start..end).await
    }

    /// Retrieves a list of key log ids that mark the beginning of each Leader.
    ///
    /// This method returns log entries that represent leadership transitions in the log history,
    /// including
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
    async fn get_key_log_ids(&mut self, range: RangeInclusive<LogIdOf<C>>) -> Result<Vec<LogIdOf<C>>, io::Error> {
        LogIdList::get_key_log_ids(range, self).await.map_err(|e| io::Error::other(e.to_string()))
    }
}
