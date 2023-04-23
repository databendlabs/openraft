use std::sync::Arc;

use openraft::async_trait::async_trait;
use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use openraft::StorageError;

use crate::store::Config;
use crate::store::LogStore;
use crate::store::NodeId;
use crate::store::StateMachineStore;

struct Builder {}
#[async_trait]
impl StoreBuilder<Config, Arc<LogStore>, Arc<StateMachineStore>> for Builder {
    async fn build(&self) -> Result<((), Arc<LogStore>, Arc<StateMachineStore>), StorageError<NodeId>> {
        let log_store = LogStore::new_async().await;
        let sm = Arc::new(StateMachineStore::new());
        Ok(((), log_store, sm))
    }
}

#[test]
pub fn test_store() -> Result<(), StorageError<NodeId>> {
    Suite::test_all(Builder {})?;
    Ok(())
}
