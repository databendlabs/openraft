use log_fjall::FjallLogStore;
use openraft::StorageError;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
use tempfile::TempDir;

use crate::FjallStateMachine;
use crate::TypeConfig;

/// Builds a fjall log store and a fjall state machine in separate databases under one temporary
/// directory, which is returned as the guard `G` and removed after the test.
struct FjallBuilder {}

impl StoreBuilder<TypeConfig, FjallLogStore<TypeConfig>, FjallStateMachine<TypeConfig>, TempDir> for FjallBuilder {
    async fn build(
        &self,
    ) -> Result<(TempDir, FjallLogStore<TypeConfig>, FjallStateMachine<TypeConfig>), StorageError<TypeConfig>> {
        let td = TempDir::new().map_err(|e| StorageError::read(TypeConfig::err_from_error(&e)))?;
        let log_store = FjallLogStore::open(td.path().join("log"))
            .map_err(|e| StorageError::read(TypeConfig::err_from_error(&e)))?;
        let sm = FjallStateMachine::open(td.path().join("sm"))
            .map_err(|e| StorageError::read(TypeConfig::err_from_error(&e)))?;
        Ok((td, log_store, sm))
    }
}

#[test]
pub fn test_fjall_store() {
    TypeConfig::run(async {
        Suite::test_all(FjallBuilder {}).await.unwrap();
    });
}
