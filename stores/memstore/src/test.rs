use std::sync::Arc;

use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use openraft::StorageError;

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

#[test]
pub fn test_mem_store() -> Result<(), StorageError<TypeConfig>> {
    Suite::test_all(MemStoreBuilder {})?;
    Ok(())
}
