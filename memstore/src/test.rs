use std::sync::Arc;

use async_trait::async_trait;
use openraft::storage::Adaptor;
use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use openraft::StorageError;

use crate::MemNodeId;
use crate::MemStore;
use crate::TypeConfig;

struct MemBuilder {}
#[async_trait]
impl StoreBuilder<TypeConfig, Adaptor<TypeConfig, Arc<MemStore>>, Adaptor<TypeConfig, Arc<MemStore>>> for MemBuilder {
    async fn build(
        &self,
    ) -> Result<
        (
            (),
            Adaptor<TypeConfig, Arc<MemStore>>,
            Adaptor<TypeConfig, Arc<MemStore>>,
        ),
        StorageError<MemNodeId>,
    > {
        let store = MemStore::new_async().await;
        let (log_store, sm) = Adaptor::new(store);
        Ok(((), log_store, sm))
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
