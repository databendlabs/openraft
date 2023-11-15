use std::fmt::Debug;
use std::ops::RangeBounds;

use macros::add_async_trait;

use crate::defensive::check_range_matches_entries;
use crate::LogId;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftLogId;
use crate::RaftLogReader;
use crate::RaftTypeConfig;
use crate::StorageError;

#[add_async_trait]
pub trait RaftLogReaderExt<C>: RaftLogReader<C>
where C: RaftTypeConfig
{
    /// Try to get an log entry.
    ///
    /// It does not return an error if the log entry at `log_index` is not found.
    async fn try_get_log_entry(&mut self, log_index: u64) -> Result<Option<C::Entry>, StorageError<C::NodeId>> {
        let mut res = self.try_get_log_entries(log_index..(log_index + 1)).await?;
        Ok(res.pop())
    }

    /// Get a series of log entries from storage.
    ///
    /// Similar to `try_get_log_entries` except an error will be returned if there is an entry not
    /// found in the specified range.
    async fn get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend + OptionalSync>(
        &mut self,
        range: RB,
    ) -> Result<Vec<C::Entry>, StorageError<C::NodeId>> {
        let res = self.try_get_log_entries(range.clone()).await?;

        check_range_matches_entries::<C, _>(range, &res)?;

        Ok(res)
    }

    /// Get the log id of the entry at `index`.
    async fn get_log_id(&mut self, log_index: u64) -> Result<LogId<C::NodeId>, StorageError<C::NodeId>> {
        let entries = self.get_log_entries(log_index..=log_index).await?;

        Ok(*entries[0].get_log_id())
    }
}

impl<C, LR> RaftLogReaderExt<C> for LR
where
    C: RaftTypeConfig,
    LR: RaftLogReader<C>,
{
}
