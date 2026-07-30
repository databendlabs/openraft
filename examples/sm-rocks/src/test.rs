use log_rocks::RocksLogStore;
use openraft::StorageError;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
use tempfile::TempDir;

use crate::RocksStateMachine;
use crate::TypeConfig;

struct RocksBuilder {}

impl StoreBuilder<TypeConfig, RocksLogStore<TypeConfig>, RocksStateMachine<TypeConfig>, TempDir> for RocksBuilder {
    async fn build(
        &self,
    ) -> Result<(TempDir, RocksLogStore<TypeConfig>, RocksStateMachine<TypeConfig>), StorageError<TypeConfig>> {
        let td = TempDir::new().map_err(|e| StorageError::read(TypeConfig::err_from_error(&e)))?;
        let log_store = RocksLogStore::open(td.path().join("log"))
            .map_err(|e| StorageError::read(TypeConfig::err_from_error(&e)))?;
        let sm = RocksStateMachine::open(td.path().join("sm"))
            .map_err(|e| StorageError::read(TypeConfig::err_from_error(&e)))?;
        Ok((td, log_store, sm))
    }
}

#[test]
pub fn test_rocks_store() {
    TypeConfig::run(async {
        Suite::test_all(RocksBuilder {}).await.unwrap();
    });
}
