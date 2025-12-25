use std::sync::Arc;

use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
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

#[test]
pub fn test_mem_store() {
    TypeConfig::run(async {
        Suite::test_all(MemKVStoreBuilder {}).await.unwrap();
    });
}
