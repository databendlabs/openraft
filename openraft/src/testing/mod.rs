mod store_builder;
mod suite;

use std::collections::BTreeSet;

use anyerror::AnyError;
pub use store_builder::StoreBuilder;
pub use suite::Suite;
use tokio::sync::oneshot;

use crate::entry::RaftEntry;
use crate::log_id::RaftLogId;
use crate::storage::LogFlushed;
use crate::storage::RaftLogStorage;
use crate::CommittedLeaderId;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StorageIOError;

/// Builds a log id, for testing purposes.
pub fn log_id<NID: crate::NodeId>(term: u64, node_id: NID, index: u64) -> LogId<NID> {
    LogId::<NID> {
        leader_id: CommittedLeaderId::new(term, node_id),
        index,
    }
}

/// Create a blank log entry for test.
pub fn blank_ent<C: RaftTypeConfig>(term: u64, node_id: C::NodeId, index: u64) -> crate::Entry<C> {
    crate::Entry::<C>::new_blank(LogId::new(CommittedLeaderId::new(term, node_id), index))
}

/// Create a membership log entry without learner config for test.
pub fn membership_ent<C: RaftTypeConfig>(
    term: u64,
    node_id: C::NodeId,
    index: u64,
    config: Vec<BTreeSet<C::NodeId>>,
) -> crate::Entry<C> {
    crate::Entry::new_membership(
        LogId::new(CommittedLeaderId::new(term, node_id), index),
        crate::Membership::new(config, None),
    )
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
