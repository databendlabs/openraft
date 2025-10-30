use openraft_macros::add_async_trait;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::core::notification::Notification;
use crate::entry::RaftEntry;
use crate::error::StorageIOResult;
use crate::raft_state::io_state::io_id::IOId;
use crate::storage::IOFlushed;
use crate::storage::RaftLogStorage;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::VoteOf;
use crate::type_config::async_runtime::mpsc::MpscReceiver;
use crate::type_config::async_runtime::mpsc::MpscSender;
use crate::vote::raft_vote::RaftVoteExt;

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

        let last_log_id = entries.last().unwrap().log_id();

        let (tx, mut rx) = C::mpsc(1024);

        let io_id = IOId::<C>::new_log_io(VoteOf::<C>::default().into_committed(), Some(last_log_id));
        let notify = Notification::LocalIO { io_id };

        let callback = IOFlushed::<C>::new(notify, tx.downgrade());
        self.append(entries, callback).await.sto_write_logs()?;

        let got = rx.recv().await.unwrap();
        if let Notification::StorageError { error } = got {
            return Err(error);
        }

        Ok(())
    }
}

impl<C, T> RaftLogStorageExt<C> for T
where
    T: RaftLogStorage<C>,
    C: RaftTypeConfig,
{
}
