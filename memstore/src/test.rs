use openraft::testing::Suite;
use openraft::StorageError;

use crate::MemNodeId;
use crate::MemStore;

#[test]
pub fn test_mem_store() -> Result<(), StorageError<MemNodeId>> {
    Suite::test_all(MemStore::new_async)
}
