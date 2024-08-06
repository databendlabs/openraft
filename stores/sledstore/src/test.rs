use std::sync::Arc;

use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::StorageError;
use tempfile::TempDir;

use crate::SledStore;
use crate::TypeConfig;

struct SledBuilder {}

#[test]
pub fn test_sled_store() -> Result<(), StorageError<TypeConfig>> {
    Suite::test_all(SledBuilder {})
}

type LogStore = Arc<SledStore>;
type StateMachine = Arc<SledStore>;

impl StoreBuilder<TypeConfig, LogStore, StateMachine, TempDir> for SledBuilder {
    async fn build(&self) -> Result<(TempDir, LogStore, StateMachine), StorageError<TypeConfig>> {
        let td = TempDir::new().expect("couldn't create temp dir");

        let db: sled::Db = sled::open(td.path()).unwrap();

        let (log_store, sm) = SledStore::new(Arc::new(db)).await;

        Ok((td, log_store, sm))
    }
}
