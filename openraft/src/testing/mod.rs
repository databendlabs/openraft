mod store_builder;
mod suite;

use anyerror::AnyError;
pub use store_builder::StoreBuilder;
pub use suite::Suite;
use tokio::sync::oneshot;

use crate::log_id::RaftLogId;
use crate::storage::LogFlushed;
use crate::storage::RaftLogStorage;
use crate::BasicNode;
use crate::CommittedLeaderId;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StorageIOError;

crate::declare_raft_types!(
    /// Dummy Raft types for the purpose of testing internal structures requiring
    /// `RaftTypeConfig`, like `MembershipConfig`.
    pub(crate) DummyConfig: D = u64, R = u64, NodeId = u64, Node = BasicNode, Entry = crate::entry::Entry<DummyConfig>
);

/// Builds a log id with node_id set to 0, for testing purposes.
pub fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: CommittedLeaderId::new(term, 1),
        index,
    }
}

/// Append to log and wait for the log to be flushed.
pub async fn blocking_append<C: RaftTypeConfig, LS: RaftLogStorage<C>, I>(
    log_store: &mut LS,
    entries: I,
) -> Result<(), StorageError<C::NodeId>>
where
    I: IntoIterator<Item = C::Entry>,
{
    let entries = entries.into_iter().collect::<Vec<_>>();
    let last_log_id = entries.last().map(|e| *e.get_log_id()).unwrap();

    let (tx, rx) = oneshot::channel();
    let cb = LogFlushed::new(Some(last_log_id), tx);
    log_store.append(entries, cb).await?;
    rx.await.unwrap().map_err(|e| StorageIOError::write_logs(AnyError::error(e)))?;

    Ok(())
}
