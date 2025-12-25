use std::sync::Arc;

use bench_minimal::store::LogStore;
use bench_minimal::store::StateMachineStore;
use bench_minimal::store::TypeConfig;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
use openraft::StorageError;

struct Builder {}

impl StoreBuilder<TypeConfig, Arc<LogStore>, Arc<StateMachineStore>> for Builder {
    async fn build(&self) -> Result<((), Arc<LogStore>, Arc<StateMachineStore>), StorageError<TypeConfig>> {
        let log_store = LogStore::new_async().await;
        let sm = Arc::new(StateMachineStore::new());
        Ok(((), log_store, sm))
    }
}

#[test]
pub fn test_store() {
    TypeConfig::run(async {
        Suite::test_all(Builder {}).await.unwrap();
    });
}
