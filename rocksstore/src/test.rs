use std::sync::Arc;

use async_trait::async_trait;
use openraft::storage::Adaptor;
use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use openraft::StorageError;
use tempfile::TempDir;

use crate::RocksNodeId;
use crate::RocksStore;
use crate::TypeConfig;

type LogStore = Adaptor<TypeConfig, Arc<RocksStore>>;
type StateMachine = Adaptor<TypeConfig, Arc<RocksStore>>;

struct RocksBuilder {}
#[async_trait]
impl StoreBuilder<TypeConfig, LogStore, StateMachine, TempDir> for RocksBuilder {
    async fn build(&self) -> Result<(TempDir, LogStore, StateMachine), StorageError<RocksNodeId>> {
        let td = TempDir::new().expect("couldn't create temp dir");
        let store = RocksStore::new(td.path()).await;
        let (log_store, sm) = Adaptor::new(store);
        Ok((td, log_store, sm))
    }
}

/// To customize a builder:
///
/// ```ignore
/// use async_trait::async_trait;
/// use openraft::testing::StoreBuilder;
/// use crate::ClientRequest;
/// use crate::ClientResponse;
///
/// struct MemStoreBuilder {}
///
/// #[async_trait]
/// impl StoreBuilder<ClientRequest, ClientResponse, MemStore> for MemStoreBuilder {
///     async fn build(&self) -> MemStore {
///         MemStore::new().await
///     }
/// }
/// #[test]
/// pub fn test_mem_store() -> anyhow::Result<()> {
///     Suite::test_all(MemStoreBuilder {})
/// }
/// ```
#[test]
pub fn test_rocks_store() -> Result<(), StorageError<RocksNodeId>> {
    Suite::test_all(RocksBuilder {})?;
    Ok(())
}
