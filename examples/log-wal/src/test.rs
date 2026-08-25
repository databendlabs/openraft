use std::sync::Arc;

use openraft::StorageError;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::BlockConfig;
use openraft_memstore::MemStateMachine;
use openraft_memstore::TypeConfig;
use tempfile::TempDir;

use crate::WalLogStore;

/// Builds a `WalLogStore` in a fresh temporary directory.
///
/// The directory is returned as the guard `G`, so it lives as long as the test
/// and is removed after it.
struct WalStoreBuilder {}

impl StoreBuilder<TypeConfig, WalLogStore<TypeConfig>, Arc<MemStateMachine>, TempDir> for WalStoreBuilder {
    async fn build(
        &self,
    ) -> Result<(TempDir, WalLogStore<TypeConfig>, Arc<MemStateMachine>), StorageError<TypeConfig>> {
        let temp_dir = TempDir::new().map_err(|e| StorageError::write(TypeConfig::err_from_error(&e)))?;

        let dir = temp_dir.path().display().to_string();
        let log_store = WalLogStore::open(dir).map_err(|e| StorageError::write(TypeConfig::err_from_error(&e)))?;

        let sm = Arc::new(MemStateMachine::new(BlockConfig::default()));

        Ok((temp_dir, log_store, sm))
    }
}

#[test]
fn test_wal_log_store() {
    TypeConfig::run(async {
        Suite::test_all(WalStoreBuilder {}).await.unwrap();
    });
}
