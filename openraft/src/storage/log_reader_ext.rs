use anyerror::AnyError;
use openraft_macros::add_async_trait;

use crate::RaftLogReader;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::entry::RaftEntry;
use crate::error::StorageIOResult;
use crate::type_config::alias::LogIdOf;

/// Extension trait for [`RaftLogReader`] providing convenience methods for log access.
#[add_async_trait]
pub trait RaftLogReaderExt<C>: RaftLogReader<C>
where C: RaftTypeConfig
{
    /// Try to get a log entry.
    ///
    /// It does not return an error if the log entry at `log_index` is not found.
    async fn try_get_log_entry(&mut self, log_index: u64) -> Result<Option<C::Entry>, StorageError<C>> {
        let mut res = self.try_get_log_entries(log_index..(log_index + 1)).await.sto_read_logs()?;
        Ok(res.pop())
    }

    /// Get the log id of the entry at `index`.
    async fn get_log_id(&mut self, log_index: u64) -> Result<LogIdOf<C>, StorageError<C>> {
        let entries = self.try_get_log_entries(log_index..=log_index).await.sto_read_logs()?;

        if entries.is_empty() {
            return Err(StorageError::read_log_at_index(
                log_index,
                AnyError::error("log entry not found"),
            ));
        }

        Ok(entries[0].log_id())
    }
}

impl<C, LR> RaftLogReaderExt<C> for LR
where
    C: RaftTypeConfig,
    LR: RaftLogReader<C> + ?Sized,
{
}
