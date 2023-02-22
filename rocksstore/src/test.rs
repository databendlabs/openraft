use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use tempdir::TempDir;

use crate::RocksRequest;
use crate::RocksResponse;
use crate::RocksStore;

struct RocksBuilder {
    temp_dir: Mutex<Vec<TempDir>>,
}

#[async_trait]
impl StoreBuilder<RocksRequest, RocksResponse, Arc<RocksStore>> for RocksBuilder {
    async fn build(&self) -> Arc<RocksStore> {
        let td = tempdir::TempDir::new("RocksBuilder").expect("couldn't create temp dir");
        let store = RocksStore::new(td.path()).await;

        {
            let mut t = self.temp_dir.lock().unwrap();
            // Do not drop
            t.push(td);
        }

        store
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
pub fn test_rocks_store() -> anyhow::Result<()> {
    Suite::test_all(RocksBuilder {
        temp_dir: Mutex::new(Vec::new()),
    })?;
    Ok(())
}
