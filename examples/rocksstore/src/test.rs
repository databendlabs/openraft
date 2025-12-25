use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
use openraft::AnyError;
use openraft::StorageError;
use tempfile::TempDir;

use crate::log_store::RocksLogStore;
use crate::RocksStateMachine;
use crate::TypeConfig;

struct RocksBuilder {}

impl StoreBuilder<TypeConfig, RocksLogStore<TypeConfig>, RocksStateMachine, TempDir> for RocksBuilder {
    async fn build(&self) -> Result<(TempDir, RocksLogStore<TypeConfig>, RocksStateMachine), StorageError<TypeConfig>> {
        let td = TempDir::new().map_err(|e| StorageError::read(AnyError::new(&e)))?;
        let (log_store, sm) = crate::new(td.path()).await.map_err(|e| StorageError::read(AnyError::new(&e)))?;
        Ok((td, log_store, sm))
    }
}

#[test]
pub fn test_rocks_store() {
    TypeConfig::run(async {
        Suite::test_all(RocksBuilder {}).await.unwrap();
    });
}
