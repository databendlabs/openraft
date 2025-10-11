use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::StorageError;
use tempfile::TempDir;

use crate::log_store::RocksLogStore;
use crate::RocksStateMachine;
use crate::TypeConfig;

struct RocksBuilder {}

impl StoreBuilder<TypeConfig, RocksLogStore<TypeConfig>, RocksStateMachine, TempDir> for RocksBuilder {
    async fn build(&self) -> Result<(TempDir, RocksLogStore<TypeConfig>, RocksStateMachine), StorageError<TypeConfig>> {
        let td = TempDir::new().map_err(|e| StorageError::read(&e))?;
        let (log_store, sm) = crate::new(td.path()).await.map_err(|e| StorageError::read(&e))?;
        Ok((td, log_store, sm))
    }
}

#[tokio::test]
pub async fn test_rocks_store() -> Result<(), StorageError<TypeConfig>> {
    Suite::test_all(RocksBuilder {}).await?;
    Ok(())
}
