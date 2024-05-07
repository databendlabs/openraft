use std::sync::Arc;

use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use openraft::StorageError;

use crate::store::{LogStore, StateMachineStore};
use crate::NodeId;
use crate::TypeConfig;

struct MemKVStoreBuilder {}

impl StoreBuilder<TypeConfig, LogStore, Arc<StateMachineStore>, ()> for MemKVStoreBuilder {
    async fn build(&self) -> Result<((), LogStore, Arc<StateMachineStore>), StorageError<NodeId>> {
        Ok(((), LogStore::default(), Arc::default()))
    }
}

#[test]
pub fn test_mem_store() -> Result<(), StorageError<NodeId>> {
    Suite::test_all(MemKVStoreBuilder {})?;
    Ok(())
}
