use std::sync::Arc;

use openraft::StorageError;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;

use crate::MemLogStore;
use crate::MemStateMachine;
use crate::TypeConfig;

struct MemStoreBuilder {}

impl StoreBuilder<TypeConfig, Arc<MemLogStore>, Arc<MemStateMachine>, ()> for MemStoreBuilder {
    async fn build(&self) -> Result<((), Arc<MemLogStore>, Arc<MemStateMachine>), StorageError<TypeConfig>> {
        let (log_store, sm) = crate::new_mem_store();
        Ok(((), log_store, sm))
    }
}

#[tokio::test]
pub async fn test_mem_store() -> Result<(), StorageError<TypeConfig>> {
    Suite::test_all(MemStoreBuilder {}).await?;
    Ok(())
}
