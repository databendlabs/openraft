use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use openraft::StorageError;

use crate::Config;
use crate::MemNodeId;
use crate::MemStore;

struct MemBuilder {}
#[async_trait]
impl StoreBuilder<Config, Arc<MemStore>> for MemBuilder {
    async fn run_test<Fun, Ret, Res>(&self, t: Fun) -> Result<Ret, StorageError<MemNodeId>>
    where
        Res: Future<Output = Result<Ret, StorageError<MemNodeId>>> + Send,
        Fun: Fn(Arc<MemStore>) -> Res + Sync + Send,
    {
        let store = MemStore::new_async().await;
        t(store).await
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
pub fn test_mem_store() -> Result<(), StorageError<MemNodeId>> {
    Suite::test_all(MemBuilder {})?;
    Ok(())
}
