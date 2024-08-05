use openraft::testing::StoreBuilder;
use openraft::testing::Suite;
use openraft::StorageError;
use tempfile::TempDir;

use crate::RocksLogStore;
use crate::RocksStateMachine;
use crate::TypeConfig;

struct RocksBuilder {}

impl StoreBuilder<TypeConfig, RocksLogStore, RocksStateMachine, TempDir> for RocksBuilder {
    async fn build(&self) -> Result<(TempDir, RocksLogStore, RocksStateMachine), StorageError<TypeConfig>> {
        let td = TempDir::new().expect("couldn't create temp dir");
        let (log_store, sm) = crate::new(td.path()).await;
        Ok((td, log_store, sm))
    }
}
/// To customize a builder:
///
/// ```ignore
/// use openraft::testing::StoreBuilder;
/// use crate::ClientRequest;
/// use crate::ClientResponse;
///
/// struct MemStoreBuilder {}
///
/// impl StoreBuilder<ClientRequest, ClientResponse, RocksLogStore, RocksStateMachine> for MemStoreBuilder {
///     async fn build(&self) -> _ {
///         // ...
///     }
/// }
/// #[test]
/// pub fn test_mem_store() -> anyhow::Result<()> {
///     let rt = YourRuntime::new();
///     rt.block_on(Suite::test_all(MemStoreBuilder {}));
/// }
/// ```
#[tokio::test]
pub async fn test_rocks_store() -> Result<(), StorageError<TypeConfig>> {
    Suite::test_all(RocksBuilder {}).await?;
    Ok(())
}
