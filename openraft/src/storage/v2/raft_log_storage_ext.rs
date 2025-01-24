use openraft_macros::add_async_trait;

use crate::async_runtime::MpscUnboundedReceiver;
use crate::async_runtime::MpscUnboundedSender;
use crate::core::notification::Notification;
use crate::entry::RaftEntry;
use crate::raft_state::io_state::io_id::IOId;
use crate::storage::IOFlushed;
use crate::storage::RaftLogStorage;
use crate::type_config::alias::VoteOf;
use crate::type_config::TypeConfigExt;
use crate::vote::raft_vote::RaftVoteExt;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::StorageError;

/// Extension trait for RaftLogStorage to provide utility methods.
///
/// All methods in this trait are provided with default implementation.
#[add_async_trait]
pub trait RaftLogStorageExt<C>: RaftLogStorage<C>
where C: RaftTypeConfig
{
    /// Blocking mode append log entries to the storage.
    ///
    /// It blocks until the callback is called by the underlying storage implementation.
    async fn blocking_append<I>(&mut self, entries: I) -> Result<(), StorageError<C>>
    where
        I: IntoIterator<Item = C::Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let entries = entries.into_iter().collect::<Vec<_>>();

        let last_log_id = entries.last().unwrap().log_id();

        let (tx, mut rx) = C::mpsc_unbounded();

        let io_id = IOId::<C>::new_log_io(VoteOf::<C>::default().into_committed(), Some(last_log_id));
        let notify = Notification::LocalIO { io_id };

        let callback = IOFlushed::<C>::new(notify, tx.downgrade());
        self.append(entries, callback).await?;

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
