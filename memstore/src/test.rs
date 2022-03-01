use openraft::testing::Suite;
use openraft::StorageError;

use crate::Config;
use crate::MemStore;

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
pub fn test_mem_store() -> Result<(), StorageError<Config>> {
    Suite::test_all(MemStore::new_async)?;
    Ok(())
}
