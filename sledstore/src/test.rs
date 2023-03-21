use std::sync::Arc;

use async_trait::async_trait;
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

#[async_trait]
impl StoreBuilder<ExampleTypeConfig, Arc<SledStore>, TempDir> for SledBuilder {
    async fn build(&self) -> Result<(TempDir, Arc<SledStore>), StorageError<ExampleNodeId>> {
        let td = tempfile::TempDir::new().expect("couldn't create temp dir");

        let db: sled::Db = sled::open(td.path()).unwrap();

        let store = SledStore::new(Arc::new(db)).await;

        Ok((td, store))
    }
}
