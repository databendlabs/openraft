use std::sync::Arc;

use async_trait::async_trait;
use openraft::storage::Adaptor;
use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use openraft::StorageError;
use tempfile::TempDir;

use crate::ExampleNodeId;
use crate::ExampleTypeConfig;
use crate::SledStore;

struct SledBuilder {}

#[test]
pub fn test_sled_store() -> Result<(), StorageError<ExampleNodeId>> {
    Suite::test_all(SledBuilder {})
}

type LogStore = Adaptor<ExampleTypeConfig, Arc<SledStore>>;
type StateMachine = Adaptor<ExampleTypeConfig, Arc<SledStore>>;

#[async_trait]
impl StoreBuilder<ExampleTypeConfig, LogStore, StateMachine, TempDir> for SledBuilder {
    async fn build(&self) -> Result<(TempDir, LogStore, StateMachine), StorageError<ExampleNodeId>> {
        let td = TempDir::new().expect("couldn't create temp dir");

        let db: sled::Db = sled::open(td.path()).unwrap();

        let store = SledStore::new(Arc::new(db)).await;
        let (log_store, sm) = Adaptor::new(store);

        Ok((td, log_store, sm))
    }
}
