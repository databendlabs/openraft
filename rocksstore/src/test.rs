use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use openraft::StorageError;

use crate::Config;
use crate::RocksNodeId;
use crate::RocksStore;

struct RocksBuilder {}
#[async_trait]
impl StoreBuilder<Config, Arc<RocksStore>> for RocksBuilder {
    async fn run_test<Fun, Ret, Res>(&self, t: Fun) -> Result<Ret, StorageError<RocksNodeId>>
    where
        Res: Future<Output = Result<Ret, StorageError<RocksNodeId>>> + Send,
        Fun: Fn(Arc<RocksStore>) -> Res + Sync + Send,
    {
        let td = tempdir::TempDir::new("RocksBuilder").expect("couldn't create temp dir");
        let r = {
            let store = RocksStore::new(td.path()).await;
            t(store).await
        };
        td.close().expect("could not close temp directory");
        r
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
pub fn test_mem_store() -> Result<(), StorageError<RocksNodeId>> {
    Suite::test_all(RocksBuilder {})?;
    Ok(())
}
