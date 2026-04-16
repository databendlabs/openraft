use openraft::testing::Suite;
use openraft::StorageError;

use crate::MemNodeId;
use crate::MemStore;

#[test]
#[allow(clippy::result_large_err)]
pub fn test_mem_store() -> Result<(), StorageError<MemNodeId>> {
    Suite::test_all(MemStore::new_async)
}
