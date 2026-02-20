use std::sync::Arc;

use openraft::StorageError;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;

use crate::MemLogStore;
use crate::MemStateMachine;
use crate::TypeConfig;

struct Builder;

impl StoreBuilder<TypeConfig, Arc<MemLogStore>, Arc<MemStateMachine>, ()> for Builder {
    async fn build(&self) -> Result<((), Arc<MemLogStore>, Arc<MemStateMachine>), StorageError<TypeConfig>> {
        let (log_store, sm) = crate::new_mem_store();
        Ok(((), log_store, sm))
    }
}

#[test]
pub fn test_mem_store() {
    TypeConfig::run(async {
        Suite::test_all(Builder).await.unwrap();
    });
}
