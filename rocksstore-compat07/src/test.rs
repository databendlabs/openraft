use std::sync::Arc;

use async_trait::async_trait;
use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use openraft::StorageError;
use tempfile::TempDir;

use crate::Config;
use crate::RocksNodeId;
use crate::RocksStore;

struct RocksBuilder {}
#[async_trait]
impl StoreBuilder<Config, Arc<RocksStore>, TempDir> for RocksBuilder {
    async fn build(&self) -> Result<(TempDir, Arc<RocksStore>), StorageError<RocksNodeId>> {
        let td = tempfile::TempDir::new().expect("couldn't create temp dir");
        let store = RocksStore::new(td.path()).await;
        Ok((td, store))
    }
}

#[test]
pub fn test_rocksstore() -> Result<(), StorageError<RocksNodeId>> {
    Suite::test_all(RocksBuilder {})?;
    Ok(())
}
