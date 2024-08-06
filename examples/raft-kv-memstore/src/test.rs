use std::sync::Arc;

use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::StorageError;

use crate::store::LogStore;
use crate::store::StateMachineStore;
use crate::TypeConfig;

struct MemKVStoreBuilder {}

impl StoreBuilder<TypeConfig, LogStore, Arc<StateMachineStore>, ()> for MemKVStoreBuilder {
    async fn build(&self) -> Result<((), LogStore, Arc<StateMachineStore>), StorageError<TypeConfig>> {
        Ok(((), LogStore::default(), Arc::default()))
    }
}

#[tokio::test]
pub async fn test_mem_store() -> Result<(), StorageError<TypeConfig>> {
    Suite::test_all(MemKVStoreBuilder {}).await?;
    Ok(())
}
