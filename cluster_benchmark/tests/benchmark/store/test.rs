use std::sync::Arc;

use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::StorageError;

use crate::store::LogStore;
use crate::store::StateMachineStore;
use crate::store::TypeConfig;

struct Builder {}

impl StoreBuilder<TypeConfig, Arc<LogStore>, Arc<StateMachineStore>> for Builder {
    async fn build(&self) -> Result<((), Arc<LogStore>, Arc<StateMachineStore>), StorageError<TypeConfig>> {
        let log_store = LogStore::new_async().await;
        let sm = Arc::new(StateMachineStore::new());
        Ok(((), log_store, sm))
    }
}

#[tokio::test]
pub async fn test_store() -> Result<(), StorageError<TypeConfig>> {
    Suite::test_all(Builder {}).await?;
    Ok(())
}
