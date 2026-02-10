use std::sync::Arc;

use openraft::RaftLogReader;
use openraft::StorageError;
use openraft::Vote;
use openraft::entry::RaftEntry;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use openraft::testing::blank_ent;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::BlockConfig;
use openraft_memstore::MemStateMachine;
use openraft_memstore::TypeConfig;
use tempfile::TempDir;

use crate::FjallLogStore;

/// Builds a `FjallLogStore` in a fresh temporary directory.
///
/// The directory is returned as the guard `G`, so it lives as long as the test
/// and is removed after it.
struct FjallStoreBuilder {}

impl StoreBuilder<TypeConfig, FjallLogStore<TypeConfig>, Arc<MemStateMachine>, TempDir> for FjallStoreBuilder {
    async fn build(
        &self,
    ) -> Result<(TempDir, FjallLogStore<TypeConfig>, Arc<MemStateMachine>), StorageError<TypeConfig>> {
        let temp_dir = TempDir::new().map_err(|e| StorageError::write(TypeConfig::err_from_error(&e)))?;

        let log_store =
            FjallLogStore::open(temp_dir.path()).map_err(|e| StorageError::write(TypeConfig::err_from_error(&e)))?;

        let sm = Arc::new(MemStateMachine::new(BlockConfig::default()));

        Ok((temp_dir, log_store, sm))
    }
}

#[test]
fn test_fjall_log_store() {
    TypeConfig::run(async {
        Suite::test_all(FjallStoreBuilder {}).await.unwrap();
    });
}

#[test]
fn test_reopen_reads_back_vote_and_logs() {
    TypeConfig::run(async {
        let temp_dir = TempDir::new().unwrap();
        let vote = Vote::new(1, 2);

        tracing::info!("--- write a vote and 5 entries, purge 2 of them, truncate 1, then close");
        {
            let mut store = FjallLogStore::<TypeConfig>::open(temp_dir.path()).unwrap();
            store.save_vote(&vote).await.unwrap();

            let (tx, rx) = TypeConfig::oneshot();
            let entries = (1..=5).map(|index| blank_ent::<TypeConfig>(1, 2, index));
            store.append(entries, IOFlushed::signal(tx)).await.unwrap();
            rx.await.unwrap().unwrap();

            store.purge(log_id::<TypeConfig>(1, 2, 2)).await.unwrap();
            store.truncate_after(Some(log_id::<TypeConfig>(1, 2, 4))).await.unwrap();
        }

        tracing::info!("--- reopen: everything written before closing is read back");
        {
            let mut store = FjallLogStore::<TypeConfig>::open(temp_dir.path()).unwrap();
            assert_eq!(Some(vote), store.read_vote().await.unwrap());

            let state = store.get_log_state().await.unwrap();
            assert_eq!(Some(log_id::<TypeConfig>(1, 2, 2)), state.last_purged_log_id);
            assert_eq!(Some(log_id::<TypeConfig>(1, 2, 4)), state.last_log_id);

            let entries = store.try_get_log_entries(..).await.unwrap();
            assert_eq!(
                vec![3, 4],
                entries.iter().map(|entry| entry.index()).collect::<Vec<_>>()
            );
        }
    });
}
