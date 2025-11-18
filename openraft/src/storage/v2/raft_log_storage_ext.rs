use anyerror::AnyError;
use openraft_macros::add_async_trait;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::error::StorageIOResult;
use crate::storage::IOFlushed;
use crate::storage::RaftLogStorage;
use crate::type_config::TypeConfigExt;

/// Extension trait for RaftLogStorage to provide utility methods.
///
/// All methods in this trait are provided with a default implementation.
#[add_async_trait]
pub trait RaftLogStorageExt<C>: RaftLogStorage<C>
where C: RaftTypeConfig
{
    /// Blocking mode appends log entries to the storage.
    ///
    /// It blocks until the callback is called by the underlying storage implementation.
    async fn blocking_append<I>(&mut self, entries: I) -> Result<(), StorageError<C>>
    where
        I: IntoIterator<Item = C::Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let entries = entries.into_iter().collect::<Vec<_>>();

        let (tx, rx) = C::oneshot();

        let callback = IOFlushed::<C>::signal(tx);
        self.append(entries, callback).await.sto_write_logs()?;

        let result = rx.await.map_err(|_e| StorageError::write_logs(AnyError::error("callback channel closed")))?;

        result.map_err(|e| StorageError::write_logs(AnyError::new(&e)))
    }
}

impl<C, T> RaftLogStorageExt<C> for T
where
    T: RaftLogStorage<C>,
    C: RaftTypeConfig,
{
}
