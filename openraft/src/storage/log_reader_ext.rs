use anyerror::AnyError;
use openraft_macros::add_async_trait;

use crate::entry::traits::RaftEntryExt;
use crate::LogId;
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
    async fn try_get_log_entry(&mut self, log_index: u64) -> Result<Option<C::Entry>, StorageError<C>> {
        let mut res = self.try_get_log_entries(log_index..(log_index + 1)).await?;
        Ok(res.pop())
    }

    /// Get the log id of the entry at `index`.
    async fn get_log_id(&mut self, log_index: u64) -> Result<LogId<C>, StorageError<C>> {
        let entries = self.try_get_log_entries(log_index..=log_index).await?;

        if entries.is_empty() {
            return Err(StorageError::read_log_at_index(
                log_index,
                AnyError::error("log entry not found"),
            ));
        }

        Ok(entries[0].to_log_id())
    }
}

impl<C, LR> RaftLogReaderExt<C> for LR
where
    C: RaftTypeConfig,
    LR: RaftLogReader<C> + ?Sized,
{
}
